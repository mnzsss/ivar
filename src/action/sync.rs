//! `ivar sync` — make the local hall match `ivar.json`.
//!
//! This is the verb people run every day, right after `git pull`. Everything
//! about it follows from that.
//!
//! **It is idempotent, and it proves it.** Every step compares before it acts
//! and reports [`Change::Unchanged`] when there was nothing to do. A sync that
//! rewrote files it did not need to would put spurious modifications in
//! `git status` on every run, and a tool that dirties your working tree for no
//! reason is a tool you stop running.
//!
//! **One bad repo does not stop the others.** A repo whose remote is
//! unreachable becomes a [`Change::Failed`] entry and a
//! [`Warning`](crate::error::Warning); the other seven still get set up, and
//! the process exits `1` rather than `2`. That is the warning discipline from
//! ARCHITECTURE.md, and this is the verb it was written for. A [`Failure`] here
//! is reserved for the cases where there is nothing to salvage: no hall, or a
//! hall whose `.ivar/` cannot be created.
//!
//! # What it does, in order
//!
//! 1. The hall skeleton — `.ivar/`, `.ivar/repos/`, and the `.gitignore` lines.
//! 2. Each repo in `ivar.json`: bare clone, default-branch worktree, setup
//!    script.
//! 3. Each provider: the managed block in its instruction file, its MCP
//!    config, and its official workflow commands — all materialised for a
//!    provider the hall lists and stripped for one it does not.
//!
//! Repos come before providers because the managed block names them, and a
//! block listing a repo that failed to clone would be describing a hall that
//! does not exist yet. It lists what `ivar.json` declares either way — the
//! manifest is the source of truth, and a transient network failure should not
//! rewrite the hall's documentation.
//!
//! # What it deliberately does not do
//!
//! **It does not fetch.** A hall with eight repos would make `sync` a
//! network-bound command that people learn to avoid, and there is already a
//! verb for it: `ivar repo pull`. `sync` reaches the network exactly once per
//! repo, the first time, to clone.
//!
//! **It does not remove repos dropped from `ivar.json`.** Deleting a worktree
//! can destroy uncommitted work, and a verb that runs on every `git pull` must
//! never be able to do that on its own. `ivar cleanup` (slice 8) is where
//! removal lives, and it will ask.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::{BranchName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
// Every helper below takes `&impl git::Git`, so the trait bound is what makes
// git's operations reachable — this module never names `git::exec` or
// `git::read`, which is the boundary the trait exists for.
use crate::git::{self, TargetState};
use crate::harness::{commands, config};
use crate::infra::{fs, hash, proc};
use crate::store::gitignore;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Repo};
use crate::store::setup_receipt::Receipt;

use super::Ctx;
use super::discover_hall;
use super::read_manifest;

/// The interpreter a setup script runs under.
///
/// Named explicitly rather than executing the script directly, so a script does
/// not need its executable bit set — a `.sh` file arriving through a `git
/// clone` on a filesystem that drops modes would otherwise fail with "permission
/// denied", which names the wrong problem. The script's own shebang is
/// advisory; this is what actually runs it.
const SETUP_INTERPRETER: &str = "bash";

/// What `ivar sync` needs.
#[derive(Debug, Clone, Default)]
pub struct SyncInput {
    /// Run every repo's setup script even when its receipt says it is current.
    /// The escape hatch for a script whose effect was undone outside `ivar` —
    /// a deleted `node_modules`, a dropped database.
    pub force_setup: bool,
}

/// What happened to one thing `sync` looked at.
///
/// Five states, and [`Self::Failed`] is the reason this is not
/// [`config::Change`]: a materialiser returns `Result` and has no failure
/// state, but a *report* has to carry one, because the whole point is that one
/// failure does not stop the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// It was not there and now is.
    Created,
    /// It was there and changed.
    Updated,
    /// It was there and was taken away.
    Removed,
    /// It was already exactly right.
    Unchanged,
    /// It could not be done. The run continued.
    Failed,
}

impl Change {
    /// The one-character marker the human surface prefixes an entry with.
    pub(crate) const fn symbol(self) -> char {
        match self {
            Self::Created => '+',
            Self::Updated => '~',
            Self::Removed => '-',
            Self::Unchanged => '=',
            Self::Failed => 'x',
        }
    }

    /// The word the counts line uses for this change.
    const fn word(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Removed => "removed",
            Self::Unchanged => "unchanged",
            Self::Failed => "failed",
        }
    }

    /// Every variant, in the order the counts line prints them.
    const ALL: [Change; 5] = [
        Change::Created,
        Change::Updated,
        Change::Removed,
        Change::Unchanged,
        Change::Failed,
    ];
}

impl From<config::Change> for Change {
    fn from(change: config::Change) -> Self {
        match change {
            config::Change::Created => Self::Created,
            config::Change::Updated => Self::Updated,
            config::Change::Unchanged => Self::Unchanged,
            config::Change::Removed => Self::Removed,
        }
    }
}

impl From<commands::Change> for Change {
    fn from(change: commands::Change) -> Self {
        match change {
            commands::Change::Created => Self::Created,
            commands::Change::Updated => Self::Updated,
            commands::Change::Removed => Self::Removed,
            commands::Change::Unchanged => Self::Unchanged,
        }
    }
}

impl From<gitignore::Changed> for Change {
    fn from(changed: gitignore::Changed) -> Self {
        match changed {
            gitignore::Changed::Created => Self::Created,
            gitignore::Changed::Yes => Self::Updated,
            gitignore::Changed::No => Self::Unchanged,
        }
    }
}

/// One line of the sync report.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// What this entry is about: `hall`, `repo api`, `claude-code`. Entries are
    /// grouped under this in the human rendering, in first-seen order.
    pub surface: String,
    /// What was looked at, within the surface.
    pub label: String,
    /// What happened to it.
    pub change: Change,
    /// Anything worth saying beyond the change — a git error, why a setup
    /// script was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Entry {
    pub(crate) fn new(
        surface: impl Into<String>,
        label: impl Into<String>,
        change: Change,
    ) -> Self {
        Self {
            surface: surface.into(),
            label: label.into(),
            change,
            detail: None,
        }
    }

    pub(crate) fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// What `ivar sync` did.
///
/// `Serialize`d as-is for `--json`; [`Self::write_human`] formats this same
/// value. See ARCHITECTURE.md, "1. `action` is the unit, and it has one output
/// shape".
#[derive(Debug, Clone, Serialize)]
pub struct SyncOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// Every step, in the order it happened.
    pub entries: Vec<Entry>,
}

impl SyncOutcome {
    /// How many entries ended in each [`Change`], keyed by the change itself.
    #[must_use]
    pub fn counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts: BTreeMap<&'static str, usize> = Change::ALL
            .iter()
            .map(|change| (change.word(), 0))
            .collect();
        for entry in &self.entries {
            *counts.entry(entry.change.word()).or_default() += 1;
        }
        counts
    }
}

impl WriteHuman for SyncOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Synced {}", self.root)?;

        // Grouped in first-seen order rather than sorted: the order things
        // happened is the order that explains them, and a repo's clone has to
        // read above the worktree that depends on it.
        let mut seen: Vec<&str> = Vec::new();
        for entry in &self.entries {
            if !seen.contains(&entry.surface.as_str()) {
                seen.push(&entry.surface);
            }
        }

        for surface in seen {
            writeln!(w)?;
            writeln!(w, "{surface}:")?;
            for entry in self.entries.iter().filter(|e| e.surface == surface) {
                match &entry.detail {
                    Some(detail) => {
                        writeln!(w, "  {} {} — {detail}", entry.change.symbol(), entry.label)?
                    }
                    None => writeln!(w, "  {} {}", entry.change.symbol(), entry.label)?,
                }
            }
        }

        let counts = self.counts();
        let rendered = Change::ALL
            .iter()
            .map(|change| {
                let word = change.word();
                format!("{word}: {}", counts.get(word).copied().unwrap_or(0))
            })
            .collect::<Vec<_>>()
            .join("  ");
        writeln!(w)?;
        writeln!(w, "{rendered}")
    }
}

/// Reconcile the hall containing [`Ctx::cwd`] against its `ivar.json`.
///
/// See the module doc comment for the order, the idempotence rule, and what
/// this deliberately does not do.
pub fn sync(ctx: &Ctx, input: SyncInput) -> Outcome<SyncOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    // The skeleton is the one part that fails the whole verb: every step below
    // writes underneath it.
    entries.push(ensure_skeleton(&layout)?);
    entries.push(Entry::new(
        "hall",
        ".gitignore",
        gitignore::ensure(&layout)?.into(),
    ));

    for repo in manifest.repos() {
        sync_repo(
            &git,
            &layout,
            repo,
            input.force_setup,
            &mut entries,
            &mut warnings,
        );
    }

    // Built once, not once per provider: the block every provider gets is the
    // same bytes, and `Provider::ALL` will only grow.
    sync_providers(&layout, &manifest, &mut entries, &mut warnings);

    Ok(Report::with_warnings(
        SyncOutcome {
            root: layout.root().to_path_buf(),
            entries,
        },
        warnings,
    ))
}

/// Regenerate every provider's managed block from `manifest`.
///
/// Shared by `ivar sync` and deregister (`repo remove --force`): both rewrite
/// the hall's provider config after the manifest changes, and the block must
/// describe the same hall in both places. Best-effort per provider — a
/// failure becomes an entry and a warning, never an abort.
///
/// The MCP config and the shipped workflow commands materialise here too: the
/// MCP config is one file at the hall root per provider (hall-scoped,
/// discovered by walk-up from every session's view dir), and the commands are
/// `/ivar-*` files in the provider's native command directory. Each is a
/// separate concern with a separate failure channel, so one provider's
/// command write failing never aborts its instruction block or another
/// provider.
pub(crate) fn sync_providers(
    layout: &Layout,
    manifest: &Manifest,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let block = config::build_block(manifest.name(), &repo_names(manifest));
    for provider in Provider::ALL {
        sync_provider(layout, manifest, provider, &block, entries, warnings);
        sync_mcp(layout, manifest, provider, entries, warnings);
        sync_commands(layout, manifest, provider, entries, warnings);
    }
}

/// Create the directories every later step writes underneath.
///
/// `.ivar/setups/` and `.ivar/skills/` are deliberately not created here. Git
/// cannot track an empty directory, so a hall that created them would promise a
/// teammate something their clone does not deliver. They come into existence
/// with their first file.
fn ensure_skeleton(layout: &Layout) -> Result<Entry, Failure> {
    let mut created = false;
    for dir in [layout.ivar_dir(), layout.repos_dir()] {
        if !fs::is_dir(&dir)? {
            created = true;
        }
        fs::ensure_dir(&dir)?;
    }

    Ok(Entry::new(
        "hall",
        ".ivar/",
        if created {
            Change::Created
        } else {
            Change::Unchanged
        },
    ))
}

/// Bring one repo's bare clone, default worktree and setup script into line.
///
/// Records what it managed before the failure: a repo whose clone landed and
/// whose setup script then failed shows both, because "the clone is there" is
/// the fact the next run depends on.
fn sync_repo(
    git: &impl git::Git,
    layout: &Layout,
    repo: &Repo,
    force_setup: bool,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let name = repo.name();
    let surface = format!("repo {name}");
    let bare = layout.repo_bare(name);
    let branch = repo.default_branch();
    let worktree = layout.repo_worktree(name, branch);

    match ensure_bare(git, repo, &bare) {
        Ok(change) => entries.push(Entry::new(&surface, "bare clone", change)),
        Err(failure) => return record_failure(entries, warnings, &surface, "bare clone", failure),
    }

    match ensure_worktree(git, &bare, &worktree, branch) {
        Ok(change) => entries.push(Entry::new(&surface, format!("worktree {branch}"), change)),
        Err(failure) => {
            return record_failure(entries, warnings, &surface, "worktree", failure);
        }
    }

    match run_setup_script(git, layout, repo, &worktree, &surface, force_setup) {
        // No script for this repo. Silence rather than a "nothing to do" line —
        // most repos will never have one, and a report is only readable if
        // every line in it is about something that exists.
        Ok(None) => {}
        Ok(Some(entry)) => entries.push(entry),
        Err(failure) => record_failure(entries, warnings, &surface, "setup script", failure),
    }
}

/// Clone `repo` into `bare` if it is not already there.
fn ensure_bare(git: &impl git::Git, repo: &Repo, bare: &Utf8Path) -> Result<Change, Failure> {
    match git.target_state(bare)? {
        TargetState::Repository => Ok(Change::Unchanged),
        TargetState::Occupied => Err(occupied(
            bare,
            "sync.bare_not_a_repository",
            "a bare clone",
            "sync.remove_partial_clone",
        )),
        TargetState::Absent => {
            if let Some(parent) = bare.parent() {
                fs::ensure_dir(parent)?;
            }
            git.clone_bare(repo.url(), bare)?;
            Ok(Change::Created)
        }
    }
}

/// Add the default-branch worktree if it is not already there.
fn ensure_worktree(
    git: &impl git::Git,
    bare: &Utf8Path,
    worktree: &Utf8Path,
    branch: &BranchName,
) -> Result<Change, Failure> {
    match git.target_state(worktree)? {
        TargetState::Repository => Ok(Change::Unchanged),
        TargetState::Occupied => Err(occupied(
            worktree,
            "sync.worktree_path_occupied",
            "a worktree",
            "sync.clear_worktree_path",
        )),
        TargetState::Absent => {
            // A branch name may contain `/`, which nests. git creates the leaf
            // itself but the intermediate directories are ours.
            if let Some(parent) = worktree.parent() {
                fs::ensure_dir(parent)?;
            }
            git.add_worktree(bare, worktree, branch.as_str())
                .map_err(|error| explain_missing_branch(git, bare, branch, error))?;
            Ok(Change::Created)
        }
    }
}

/// Something is at `path` and git does not recognise it: a clone that died
/// partway, or a directory someone made by hand.
///
/// Both callers say this, differing only in what they expected to find, so it
/// is said once. Letting `git clone` (or `git worktree add`) refuse the
/// non-empty target instead would name the symptom and not the cause — and
/// removing the directory is not a call a verb that runs on every `git pull`
/// gets to make on its own, which is why the fix is marked unsafe.
fn occupied(
    path: &Utf8Path,
    code: &'static str,
    expected: &str,
    fix_code: &'static str,
) -> Failure {
    Failure::blocked(code, format!("`{path}` exists but is not {expected}"))
        .expected(format!("{expected}, or nothing at all"))
        .actual("a directory that git does not recognise")
        .fix(FixAction::unsafe_(
            fix_code,
            format!(
                "Remove `{path}` and run `ivar sync` again — check first that nothing of yours is in it."
            ),
        ))
}

/// Turn `git worktree add`'s refusal into something a user can act on when the
/// cause is that the manifest names a branch this repository does not have.
///
/// The common shape of this: `ivar.json` says `main`, the remote's default is
/// `master`. git's own message ("invalid reference") names neither the manifest
/// nor the branch that *does* exist, so the user is left guessing at which of
/// the two is wrong. Asking the bare clone what its `HEAD` points at costs one
/// local read and turns that into a sentence.
///
/// Falls back to git's original error whenever the answer would not help — the
/// branch matches the default (so the cause is something else), or `HEAD`
/// itself will not answer.
fn explain_missing_branch(
    git: &impl git::Git,
    bare: &Utf8Path,
    branch: &BranchName,
    error: git::Error,
) -> Failure {
    let Ok(default) = git.head_branch(bare) else {
        return error.into();
    };
    if default == branch.as_str() {
        return error.into();
    }

    // The default branch goes in `what`, not only in `actual`: a per-item
    // failure reaches the user through `record_failure`, which keeps the
    // sentence and drops everything around it. A message whose useful half
    // lives in a field nobody renders is a message that did not get delivered.
    Failure::blocked(
        "sync.branch_not_in_repo",
        format!("`{branch}` is not a branch in this repository; its default branch is `{default}`"),
    )
    .expected(format!("a branch named `{branch}`, as `ivar.json` declares"))
    .actual(format!("this repository's default branch is `{default}`"))
    .fix(FixAction::safe(
        "sync.correct_default_branch",
        format!("Set this repo's `default_branch` to `{default}` in `ivar.json`, then run `ivar sync` again."),
    ))
}

/// Run this repo's setup script in its default worktree, if there is one and it
/// needs running.
///
/// `Ok(None)` means the repo has no setup script — the common case, and not
/// worth a line in the report.
///
/// The script's output is **streamed, not captured**: a `pnpm install` is
/// minutes long, and a user watching a frozen progress line has no way to tell
/// a slow install from a hang.
/// Run this repo's setup script in its default worktree, if there is one and it
/// needs running.
///
/// `Ok(None)` means the repo has no setup script — the common case, and not
/// worth a line in the report.
///
/// The script's output is **streamed, not captured**: a `pnpm install` is
/// minutes long, and a user watching a frozen progress line has no way to tell
/// a slow install from a hang.
pub(crate) fn run_setup_script(
    git: &impl git::Git,
    layout: &Layout,
    repo: &Repo,
    worktree: &Utf8Path,
    surface: &str,
    force: bool,
) -> Result<Option<Entry>, Failure> {
    let script = layout.setup_script(repo.name());
    if !fs::is_file(&script)? {
        return Ok(None);
    }

    // Content, not mtime: a `git pull` that rewrites the script byte-identical
    // must not trigger a re-run, and one that changes a line must.
    let fingerprint = hash::file(&script)?;
    let git_dir = git.worktree_git_dir(worktree)?;
    let receipt = Receipt::read(&git_dir);

    if !Receipt::should_run(receipt.as_ref(), &fingerprint, force) {
        return Ok(Some(
            Entry::new(surface, "setup script", Change::Unchanged)
                .detail("already run for this version of the script"),
        ));
    }
    let first_run = receipt.is_none();

    let code = proc::inherit(&setup_command(layout, repo, worktree, &script))?;

    // Recorded before the exit code is judged, so a failed run is remembered as
    // failed — which is what makes the next sync retry it instead of skipping.
    Receipt::write(&git_dir, &Receipt::of_run(&fingerprint, code))?;

    if code != Some(0) {
        return Err(setup_script_failed(&script, code));
    }

    Ok(Some(Entry::new(
        surface,
        "setup script",
        if first_run {
            Change::Created
        } else {
            Change::Updated
        },
    )))
}

/// The setup script's invocation, carrying the `IVAR_*` environment contract
/// from ARCHITECTURE.md.
///
/// These names are a **public contract**: a user's committed
/// `.ivar/setups/<repo>.sh` breaks if they move. `IVAR_WORKTREE` duplicates the
/// working directory on purpose — a script that `cd`s somewhere still needs a
/// way back.
fn setup_command(
    layout: &Layout,
    repo: &Repo,
    worktree: &Utf8Path,
    script: &Utf8Path,
) -> proc::Command {
    proc::Command::new(SETUP_INTERPRETER)
        .arg(script.as_str())
        .cwd(worktree)
        .env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_REPO", repo.name().as_str())
        .env("IVAR_BRANCH", repo.default_branch().as_str())
        .env("IVAR_WORKTREE", worktree.as_str())
        // `default`, never `feature`: this is the hall's own checkout of the
        // repo's default branch. Feature worktrees get theirs on promote.
        .env("IVAR_WORKTREE_KIND", "default")
}

fn setup_script_failed(script: &Utf8Path, code: Option<i32>) -> Failure {
    let ended = match code {
        Some(code) => format!("exited {code}"),
        None => "was killed by a signal".to_owned(),
    };

    Failure::failed("sync.setup_script_failed", format!("`{script}` {ended}"))
        .expected("the setup script to exit 0")
        .actual(ended)
        .fix(FixAction::safe(
            "sync.read_setup_output",
            "Read the script's output above — it ran with its own stdout and stderr attached.",
        ))
        .fix(
            FixAction::safe(
                "sync.retry_setup",
                "Fix the script, then run `ivar sync` again — a failed run is always retried.",
            )
            .command("ivar sync"),
        )
}

/// The repo names the managed block lists, in manifest order.
fn repo_names(manifest: &Manifest) -> Vec<RepoName> {
    manifest
        .repos()
        .iter()
        .map(|repo| repo.name().clone())
        .collect()
}

/// Materialise (or strip) `provider`'s managed block.
///
/// `block` is what a provider the hall *lists* should end up holding; a
/// provider it does not list has its block stripped and `block` goes unused.
fn sync_provider(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    block: &str,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let path = layout.instruction_file(&provider);
    let label = format!("{} managed block", provider.instruction_file());

    let result = if manifest.providers().available().contains(&provider) {
        config::materialise(&path, block)
    } else {
        config::remove(&path)
    };

    match result {
        Ok(change) => entries.push(Entry::new(provider.id(), label, change.into())),
        Err(error) => record_failure(entries, warnings, provider.id(), &label, error.into()),
    }
}

/// Materialise (or strip) `provider`'s MCP config at the hall root.
///
/// The MCP config is hall-scoped — one file at the root per provider,
/// discovered by walk-up from every session's view dir — so it is regenerated
/// on every sync next to the instruction-file block, from whatever the
/// manifest declares (empty for v1). A provider the hall lists gets its config
/// written; one it does not list has the MCP key stripped, exactly like the
/// managed block.
fn sync_mcp(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let path = layout.mcp_config(&provider);
    let label = format!("{} MCP config", provider.mcp_config_path());

    let result = if manifest.providers().available().contains(&provider) {
        config::materialise_mcp(&path, provider, manifest.mcp_servers())
    } else {
        config::remove_mcp(&path, provider)
    };

    match result {
        Ok(change) => entries.push(Entry::new(provider.id(), label, change.into())),
        Err(error) => record_failure(entries, warnings, provider.id(), &label, error.into()),
    }
}

/// Materialise (or strip) `provider`'s shipped workflow commands.
///
/// A provider the hall lists gets its `/ivar-*` commands reconciled against
/// the embedded catalog — created, repaired, and cleaned of anything else in
/// the reserved namespace; one it does not list has them all removed. Every
/// other file in the command directory belongs to the user and survives
/// either way. Best-effort like the block and MCP steps: a failure becomes a
/// `Failed` entry and a warning, and the other providers still finish.
fn sync_commands(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
) {
    let path = layout.commands_dir(&provider);
    let result = if manifest.providers().available().contains(&provider) {
        commands::materialise(&path)
    } else {
        commands::remove(&path)
    };

    match result {
        Ok(changes) => {
            for change in changes {
                entries.push(Entry::new(
                    provider.id(),
                    format!("command {}", change.file_name),
                    change.change.into(),
                ));
            }
        }
        Err(error) => {
            record_failure(entries, warnings, provider.id(), "official commands", error.into());
        }
    }
}

/// Turn a step's [`Failure`] into a report entry plus a warning, and keep going.
///
/// The warning is what makes the process exit `1` instead of `0`; the entry is
/// what tells the user which step it was. The failure's `fix_actions` do not
/// survive into the warning — [`Warning`] has no room for them — which is a
/// real loss, and the reason the entry keeps the sentence.
fn record_failure(
    entries: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
    surface: &str,
    label: &str,
    failure: Failure,
) {
    entries.push(Entry::new(surface, label, Change::Failed).detail(failure.what.clone()));
    warnings.push(Warning::new("sync.step_failed", surface, failure.what));
}

/// Materialise `provider`'s shipped workflow commands during setup, or the
/// warning to carry when they cannot be written.
///
/// Shared by `init` and `provider add`: both write the manifest first and then
/// bootstrap the newly selected provider's commands immediately. A write
/// failure is a warning — the manifest stays valid and `ivar sync` is the
/// repair — never a failure that rolls the setup back.
pub(crate) fn materialise_commands(
    layout: &Layout,
    provider: Provider,
) -> Result<(), Warning> {
    commands::materialise(&layout.commands_dir(&provider)).map(|_| ()).map_err(|error| {
        Warning::new(
            "provider.commands_not_materialised",
            provider.id(),
            format!("official commands could not be written: {error}; run `ivar sync` to repair"),
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::hall::{self, InitInput};
    use crate::domain::mcp::McpServerDef;
    use crate::domain::name::HallName;
    use crate::error::Status;
    use crate::store::manifest::Providers;
    use crate::test_support::{hall_root, seeded_repo};

    /// A hall with `repos` already declared in its `ivar.json`, plus the
    /// origins those repos point at. Returns the hall root and the scratch dir
    /// guard.
    fn hall_with(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();

        if !repos.is_empty() {
            let origins = root.parent().unwrap().join("origins");
            let declared: Vec<Repo> = repos
                .iter()
                .map(|(name, branch)| {
                    let origin = seeded_repo(&origins.join(name), branch);
                    Repo::new(
                        RepoName::new(*name).unwrap(),
                        origin.as_str(),
                        BranchName::new(*branch).unwrap(),
                    )
                })
                .collect();

            let layout = Layout::at(root.clone());
            let manifest = Manifest::new(
                HallName::new("acme").unwrap(),
                Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
                declared,
                None,
            )
            .unwrap();
            Manifest::write(&layout, &manifest).unwrap();
        }

        (guard, root)
    }

    fn entry<'a>(outcome: &'a SyncOutcome, surface: &str, label: &str) -> &'a Entry {
        outcome
            .entries
            .iter()
            .find(|e| e.surface == surface && e.label == label)
            .unwrap_or_else(|| panic!("no `{surface}` / `{label}` entry in {:?}", outcome.entries))
    }

    // -- the empty hall --------------------------------------------------------

    #[test]
    fn syncing_a_hall_with_no_repos_sets_up_the_skeleton_and_the_managed_block() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        assert!(fs::is_dir(&root.join(".ivar/repos")).unwrap());
        assert_eq!(
            entry(&report.value, "claude-code", "CLAUDE.md managed block").change,
            Change::Created
        );
        let block = fs::read_text(&root.join("CLAUDE.md")).unwrap().unwrap();
        assert!(block.contains("# acme"));
    }

    /// `sync` runs after every `git pull`. The second run must touch nothing,
    /// or every run leaves a spurious modification in `git status`.
    #[test]
    fn a_second_sync_changes_nothing() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();
        let before = fs::read_bytes(&root.join("CLAUDE.md")).unwrap().unwrap();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        assert!(
            report
                .value
                .entries
                .iter()
                .all(|e| e.change == Change::Unchanged),
            "expected every entry unchanged, got {:?}",
            report.value.entries
        );
        assert_eq!(
            fs::read_bytes(&root.join("CLAUDE.md")).unwrap().unwrap(),
            before
        );
    }

    // -- repos -----------------------------------------------------------------

    #[test]
    fn a_declared_repo_is_cloned_bare_and_gets_its_default_branch_worktree() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        assert_eq!(
            entry(&report.value, "repo api", "bare clone").change,
            Change::Created
        );
        assert_eq!(
            entry(&report.value, "repo api", "worktree main").change,
            Change::Created
        );
        assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join(".ivar/repos/api/main/README.md")).unwrap(),
            "seed\n"
        );
    }

    #[test]
    fn the_managed_block_lists_every_declared_repo() {
        let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
        let ctx = Ctx::new(root.clone());

        sync(&ctx, SyncInput::default()).unwrap();

        let block = fs::read_text(&root.join("CLAUDE.md")).unwrap().unwrap();
        assert!(block.contains("`api`"));
        assert!(block.contains("`web`"));
    }

    /// The whole point of the warning channel: eight repos, one bad remote,
    /// seven still set up and the process exits 1 rather than 2.
    #[test]
    fn an_unreachable_repo_becomes_a_warning_and_the_others_still_sync() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let layout = Layout::at(root.clone());
        let mut repos = Manifest::read(&layout).unwrap().unwrap().repos().to_vec();
        repos.push(Repo::new(
            RepoName::new("ghost").unwrap(),
            root.join("no-such-origin").as_str(),
            BranchName::new("main").unwrap(),
        ));
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            repos,
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(!report.is_clean(), "a failed repo must not be a clean run");
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].subject, "repo ghost");
        assert_eq!(
            entry(&report.value, "repo ghost", "bare clone").change,
            Change::Failed
        );
        // The healthy repo still landed.
        assert_eq!(
            entry(&report.value, "repo api", "bare clone").change,
            Change::Created
        );
        assert!(root.join(".ivar/repos/api/main/README.md").is_file());
    }

    /// git's own message for a non-empty clone target names the symptom, not
    /// the cause. This says what is actually wrong and what to do about it.
    #[test]
    fn a_partial_clone_left_at_the_bare_path_is_named_rather_than_left_to_git() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let bare = root.join(".ivar/repos/api/.bare");
        fs::ensure_dir(&bare).unwrap();
        fs::write_text(&bare.join("leftover"), "junk").unwrap();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        let failed = entry(&report.value, "repo api", "bare clone");
        assert_eq!(failed.change, Change::Failed);
        assert!(
            failed
                .detail
                .as_deref()
                .unwrap()
                .contains("is not a bare clone"),
            "was: {:?}",
            failed.detail
        );
    }

    /// The worktree twin of the case above. Both go through `occupied`, so this
    /// is what keeps the shared helper honest about saying the *right* noun.
    #[test]
    fn something_else_at_the_worktree_path_is_named_rather_than_left_to_git() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let worktree = root.join(".ivar/repos/api/main");
        fs::ensure_dir(&worktree).unwrap();
        fs::write_text(&worktree.join("notes.md"), "mine").unwrap();
        let ctx = Ctx::new(root);

        let report = sync(&ctx, SyncInput::default()).unwrap();

        let failed = entry(&report.value, "repo api", "worktree");
        assert_eq!(failed.change, Change::Failed);
        assert!(
            failed
                .detail
                .as_deref()
                .unwrap()
                .contains("is not a worktree"),
            "was: {:?}",
            failed.detail
        );
    }

    /// `main` versus `master` is the commonest first-run mistake, and git's own
    /// refusal names neither the manifest nor the branch that does exist.
    #[test]
    fn a_branch_the_repo_does_not_have_names_the_repos_default_instead() {
        let (_guard, root) = hall_with(&[("api", "master")]);
        // The origin is on `master`; declare `main` in the manifest instead.
        let layout = Layout::at(root.clone());
        let url = Manifest::read(&layout).unwrap().unwrap().repos()[0]
            .url()
            .to_owned();
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                url,
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
        let ctx = Ctx::new(root);

        let report = sync(&ctx, SyncInput::default()).unwrap();

        let failed = entry(&report.value, "repo api", "worktree");
        assert_eq!(failed.change, Change::Failed);
        let detail = failed.detail.as_deref().unwrap();
        assert!(detail.contains("main"), "was: {detail}");
        assert!(
            detail.contains("master"),
            "the branch that DOES exist has to survive into the rendered sentence, \
             not sit in a field a per-item failure never renders: {detail}"
        );
        assert!(
            report.warnings.iter().any(|w| w.subject == "repo api"),
            "a named branch mismatch must still be a warning, not a silent skip"
        );
    }

    // -- setup scripts ---------------------------------------------------------

    /// A setup script writes something a `git clone` never would — the whole
    /// reason the hook exists.
    fn write_setup_script(root: &Utf8Path, repo: &str, body: &str) {
        let script = Layout::at(root).setup_script(&RepoName::new(repo).unwrap());
        fs::ensure_dir(script.parent().unwrap()).unwrap();
        fs::write_text(&script, body).unwrap();
    }

    #[test]
    fn a_repos_setup_script_runs_in_its_worktree_with_the_ivar_environment() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        write_setup_script(
            &root,
            "api",
            "#!/usr/bin/env bash\nset -euo pipefail\n\
             printf '%s %s %s\\n' \"$IVAR_REPO\" \"$IVAR_BRANCH\" \"$IVAR_WORKTREE_KIND\" > .ivar-setup-ran\n",
        );
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        assert_eq!(
            entry(&report.value, "repo api", "setup script").change,
            Change::Created
        );
        let evidence = root.join(".ivar/repos/api/main/.ivar-setup-ran");
        assert_eq!(
            std::fs::read_to_string(&evidence).unwrap(),
            "api main default\n"
        );
    }

    #[test]
    fn a_setup_script_does_not_run_twice_for_the_same_content() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        write_setup_script(
            &root,
            "api",
            "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
        );
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "repo api", "setup script").change,
            Change::Unchanged
        );
        let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
        assert_eq!(std::fs::read_to_string(&runs).unwrap(), "x");
    }

    #[test]
    fn changing_the_script_makes_it_run_again() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        write_setup_script(
            &root,
            "api",
            "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
        );
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();

        write_setup_script(
            &root,
            "api",
            "#!/usr/bin/env bash\nprintf y >> .ivar-setup-runs\n",
        );
        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "repo api", "setup script").change,
            Change::Updated
        );
        let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
        assert_eq!(std::fs::read_to_string(&runs).unwrap(), "xy");
    }

    #[test]
    fn force_setup_runs_an_unchanged_script_again() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        write_setup_script(
            &root,
            "api",
            "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
        );
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();

        sync(&ctx, SyncInput { force_setup: true }).unwrap();

        let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
        assert_eq!(std::fs::read_to_string(&runs).unwrap(), "xx");
    }

    /// A failed setup that recorded "done" would leave every later sync
    /// silently skipping the repair the user is waiting for.
    #[test]
    fn a_failing_setup_script_warns_and_is_retried_on_the_next_sync() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        write_setup_script(
            &root,
            "api",
            "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\nexit 1\n",
        );
        let ctx = Ctx::new(root.clone());

        let first = sync(&ctx, SyncInput::default()).unwrap();
        assert!(!first.is_clean());
        assert_eq!(
            entry(&first.value, "repo api", "setup script").change,
            Change::Failed
        );

        let second = sync(&ctx, SyncInput::default()).unwrap();
        assert_eq!(
            entry(&second.value, "repo api", "setup script").change,
            Change::Failed
        );
        let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
        assert_eq!(
            std::fs::read_to_string(&runs).unwrap(),
            "xx",
            "a failed setup must be retried, not remembered as done"
        );
    }

    #[test]
    fn a_repo_with_no_setup_script_produces_no_setup_entry() {
        let (_guard, root) = hall_with(&[("api", "main")]);
        let ctx = Ctx::new(root);

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(
            !report
                .value
                .entries
                .iter()
                .any(|e| e.label == "setup script"),
            "expected no setup entry, got {:?}",
            report.value.entries
        );
    }

    // -- providers -------------------------------------------------------------

    #[test]
    fn a_provider_the_hall_does_not_list_has_its_block_stripped() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());
        // A stale block from when the hall did list OpenCode.
        let agents = root.join("AGENTS.md");
        fs::write_text(
            &agents,
            &format!(
                "{}\nstale\n{}\n\n# House rules\n",
                config::MANAGED_START,
                config::MANAGED_END
            ),
        )
        .unwrap();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "opencode", "AGENTS.md managed block").change,
            Change::Removed
        );
        assert_eq!(
            fs::read_text(&agents).unwrap().unwrap(),
            "# House rules\n",
            "the user's own text must survive"
        );
    }

    #[test]
    fn a_provider_the_hall_does_not_list_and_never_did_is_unchanged() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "opencode", "AGENTS.md managed block").change,
            Change::Unchanged
        );
        assert!(!fs::exists(&root.join("AGENTS.md")).unwrap());
    }

    // -- MCP config -----------------------------------------------------------

    #[test]
    fn sync_materialises_the_mcp_config_at_the_hall_root() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        assert_eq!(
            entry(&report.value, "claude-code", ".mcp.json MCP config").change,
            Change::Created
        );
        let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
        // Valid JSON, matching the empty-server v1 shape.
        let parsed: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
        assert_eq!(parsed, serde_json::json!({ "mcpServers": {} }));
    }

    /// `sync` runs after every `git pull`; the second run must leave the MCP
    /// config byte-identical too.
    #[test]
    fn the_mcp_config_is_unchanged_on_a_second_sync() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();
        let before = fs::read_bytes(&root.join(".mcp.json")).unwrap().unwrap();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "claude-code", ".mcp.json MCP config").change,
            Change::Unchanged
        );
        assert_eq!(
            fs::read_bytes(&root.join(".mcp.json")).unwrap().unwrap(),
            before
        );
    }

    #[test]
    fn sync_materialises_the_opencode_config_when_opencode_is_available() {
        let (_guard, root) = hall_with(&[]);
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(
                vec![Provider::ClaudeCode, Provider::OpenCode],
                Provider::ClaudeCode,
            ),
            vec![],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "opencode", "opencode.json MCP config").change,
            Change::Created
        );
        let on_disk = fs::read_text(&root.join("opencode.json")).unwrap().unwrap();
        assert!(on_disk.contains("$schema"), "was: {on_disk}");
        assert!(on_disk.contains("\"mcp\": {}"), "was: {on_disk}");
    }

    #[test]
    fn sync_strips_a_stale_mcp_config_for_a_provider_the_hall_dropped() {
        let (_guard, root) = hall_with(&[]);
        // A stale opencode.json from when the hall did list OpenCode — carrying
        // a user key that must survive the strip.
        fs::write_text(
            &root.join("opencode.json"),
            &serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "model": "anthropic/claude-sonnet-4-5",
                "mcp": { "stale": { "type": "local", "command": ["old"] } },
            })
            .to_string(),
        )
        .unwrap();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "opencode", "opencode.json MCP config").change,
            Change::Removed
        );
        let on_disk = fs::read_text(&root.join("opencode.json")).unwrap().unwrap();
        assert!(on_disk.contains("claude-sonnet-4-5"), "was: {on_disk}");
        assert!(!on_disk.contains("stale"), "was: {on_disk}");
    }

    #[test]
    fn sync_writes_declared_servers_into_the_config() {
        let (_guard, root) = hall_with(&[]);
        let layout = Layout::at(root.clone());
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        let manifest = manifest
            .with_mcp_servers(vec![McpServerDef::new("docs", "stdio").command("npx")])
            .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
        assert!(on_disk.contains("\"docs\""), "was: {on_disk}");
        assert!(on_disk.contains("\"npx\""), "was: {on_disk}");
    }

    // -- official workflow commands -------------------------------------------

    /// A hall whose `ivar.json` lists both providers.
    fn hall_with_both_providers() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_with(&[]);
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(
                vec![Provider::ClaudeCode, Provider::OpenCode],
                Provider::ClaudeCode,
            ),
            vec![],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
        (guard, root)
    }

    /// The embedded source of the shipped command `id`.
    fn embedded(id: &str) -> String {
        commands::catalog()
            .iter()
            .find(|command| command.id == id)
            .unwrap()
            .content
            .to_owned()
    }

    #[test]
    fn sync_materialises_shipped_commands_for_available_providers() {
        let (_guard, root) = hall_with_both_providers();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(report.is_clean());
        for provider in Provider::ALL {
            let dir = root.join(provider.commands_dir());
            for command in commands::catalog() {
                assert!(
                    fs::is_file(&dir.join(command.file_name())).unwrap(),
                    "{} missing for {provider}",
                    command.file_name()
                );
            }
        }
        // OpenCode's commands come from this sync — init only bootstrapped the
        // default provider, Claude Code.
        assert_eq!(
            entry(&report.value, "opencode", "command ivar-plan.md").change,
            Change::Created
        );
    }

    #[test]
    fn second_sync_reports_commands_unchanged_without_rewriting() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();

        let dir = root.join(".claude/commands");
        let before: Vec<(Utf8PathBuf, Vec<u8>, Option<std::time::SystemTime>)> = commands::catalog()
            .iter()
            .map(|command| {
                let path = dir.join(command.file_name());
                let bytes = fs::read_bytes(&path).unwrap().unwrap();
                let mtime = std::fs::metadata(path.as_std_path())
                    .ok()
                    .and_then(|metadata| metadata.modified().ok());
                (path, bytes, mtime)
            })
            .collect();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        for (path, before_bytes, before_mtime) in &before {
            assert_eq!(fs::read_bytes(path).unwrap().unwrap(), *before_bytes, "{path}");
            let mtime = std::fs::metadata(path.as_std_path())
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            assert_eq!(&mtime, before_mtime, "{path} must not be rewritten");
        }
        assert_eq!(
            entry(&report.value, "claude-code", "command ivar-plan.md").change,
            Change::Unchanged
        );
    }

    #[test]
    fn sync_repairs_modified_shipped_command_and_preserves_custom_command() {
        let (_guard, root) = hall_with(&[]);
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();

        let custom = root.join(".claude/commands/custom.md");
        fs::write_text(&custom, "mine\n").unwrap();
        fs::write_text(&root.join(".claude/commands/ivar-plan.md"), "changed\n").unwrap();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "claude-code", "command ivar-plan.md").change,
            Change::Updated
        );
        assert_eq!(
            fs::read_text(&root.join(".claude/commands/ivar-plan.md"))
                .unwrap()
                .unwrap(),
            embedded("plan")
        );
        assert_eq!(fs::read_text(&custom).unwrap().unwrap(), "mine\n");
    }

    #[test]
    fn sync_removes_only_shipped_commands_for_unavailable_provider() {
        let (_guard, root) = hall_with_both_providers();
        let ctx = Ctx::new(root.clone());
        sync(&ctx, SyncInput::default()).unwrap();

        let custom = root.join(".opencode/commands/custom.md");
        fs::write_text(&custom, "mine\n").unwrap();

        // Drop OpenCode from the manifest.
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(
            entry(&report.value, "opencode", "command ivar-plan.md").change,
            Change::Removed
        );
        assert!(
            !fs::exists(&root.join(".opencode/commands/ivar-plan.md")).unwrap(),
            "a dropped provider's shipped commands must be removed"
        );
        assert_eq!(
            fs::read_text(&custom).unwrap().unwrap(),
            "mine\n",
            "the user's command must survive provider removal"
        );
    }

    /// Deterministic by construction: a regular file occupying the parent path
    /// refuses `ensure_dir` regardless of whether the test runs as root —
    /// permission bits would not be.
    #[test]
    fn command_write_failure_warns_and_other_provider_steps_continue() {
        let (_guard, root) = hall_with_both_providers();
        fs::write_text(&root.join(".opencode"), "not a directory\n").unwrap();
        let ctx = Ctx::new(root.clone());

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert!(!report.is_clean(), "a failed command write must not be clean");
        assert!(
            report
                .value
                .entries
                .iter()
                .any(|e| e.surface == "opencode"
                    && e.label == "official commands"
                    && e.change == Change::Failed),
            "expected a failed opencode commands entry in {:?}",
            report.value.entries
        );
        assert!(
            report.warnings.iter().any(|warning| warning.subject == "opencode"),
            "expected an opencode warning in {:?}",
            report.warnings
        );
        // The other provider's commands and config completed regardless.
        assert!(fs::is_file(&root.join(".claude/commands/ivar-plan.md")).unwrap());
        assert!(fs::is_file(&root.join("CLAUDE.md")).unwrap());
        // OpenCode's own non-command config still landed.
        assert!(fs::is_file(&root.join("AGENTS.md")).unwrap());
        assert!(fs::is_file(&root.join("opencode.json")).unwrap());
    }

    // -- not in a hall ---------------------------------------------------------

    #[test]
    fn syncing_outside_a_hall_is_blocked_and_points_at_init() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = sync(&ctx, SyncInput::default()).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "hall.not_found");
        assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar init"));
    }

    #[test]
    fn sync_works_from_a_subdirectory_of_the_hall() {
        let (_guard, root) = hall_with(&[]);
        let nested = root.join("deep/inside");
        fs::ensure_dir(&nested).unwrap();
        let ctx = Ctx::new(nested);

        let report = sync(&ctx, SyncInput::default()).unwrap();

        assert_eq!(report.value.root, root);
    }

    // -- rendering -------------------------------------------------------------

    #[test]
    fn the_human_surface_groups_by_surface_and_ends_with_the_counts() {
        let outcome = SyncOutcome {
            root: Utf8PathBuf::from("/hall"),
            entries: vec![
                Entry::new("hall", ".ivar/", Change::Unchanged),
                Entry::new("repo api", "bare clone", Change::Created),
                Entry::new("repo api", "worktree main", Change::Failed).detail("branch not found"),
            ],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Synced /hall\n\
             \n\
             hall:\n\
             \x20 = .ivar/\n\
             \n\
             repo api:\n\
             \x20 + bare clone\n\
             \x20 x worktree main — branch not found\n\
             \n\
             created: 1  updated: 0  removed: 0  unchanged: 1  failed: 1\n"
        );
    }

    #[test]
    fn the_json_surface_carries_every_entry_and_its_change() {
        let outcome = SyncOutcome {
            root: Utf8PathBuf::from("/hall"),
            entries: vec![Entry::new("hall", ".ivar/", Change::Created)],
        };

        let json = serde_json::to_string(&Report::new(outcome)).unwrap();

        assert_eq!(
            json,
            r#"{"root":"/hall","entries":[{"surface":"hall","label":".ivar/","change":"created"}]}"#
        );
    }
}
