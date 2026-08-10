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
///
/// `.ivar/secrets/` is created, and the difference is the reason: it is local,
/// gitignored, and reaches no teammate's clone, so there is nothing to promise
/// falsely. Creating it is how a user finds out where `IVAR_SECRETS_DIR` points
/// without reading the docs.
fn ensure_skeleton(layout: &Layout) -> Result<Entry, Failure> {
    let mut created = false;
    for dir in [layout.ivar_dir(), layout.repos_dir(), layout.secrets_dir()] {
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
        .env("IVAR_SECRETS_DIR", layout.secrets_dir().as_str())
        // `default`, never `feature`: this is the hall's own checkout of the
        // repo's default branch. Feature worktrees get theirs on promote.
        .env("IVAR_WORKTREE_KIND", "default")
    // `IVAR_FEATURE` is deliberately absent: there is no feature here. The
    // promote path sets it, and a script that reads it unguarded should fail
    // loudly on the default worktree rather than silently bootstrap the wrong
    // thing.
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
            record_failure(
                entries,
                warnings,
                provider.id(),
                "official commands",
                error.into(),
            );
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
pub(crate) fn materialise_commands(layout: &Layout, provider: Provider) -> Result<(), Warning> {
    commands::materialise(&layout.commands_dir(&provider))
        .map(|_| ())
        .map_err(|error| {
            Warning::new(
                "provider.commands_not_materialised",
                provider.id(),
                format!(
                    "official commands could not be written: {error}; run `ivar sync` to repair"
                ),
            )
        })
}

#[cfg(test)]
#[path = "../../tests/unit/action/sync.rs"]
mod tests;
