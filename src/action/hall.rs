//! `hall`: the verbs that act on the hall itself — `init · status · doctor ·
//! cleanup · migrate`. See ARCHITECTURE.md's module map.

use std::io;
use std::io::Write;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::health::{Health, RepoHealth};
use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::fs;
use crate::store::gitignore;
use crate::store::layout::{self, Layout};
use crate::store::manifest::{Manifest, MigrationPlan, Providers};

use super::Ctx;

/// The provider a hall gets when nobody names one: Claude Code. It is the
/// harness this tool's own docs are written for, and it is what
/// `store::manifest`'s own sample data already defaults to — picking
/// anything else as the *silent* default would be a second, undocumented
/// answer to the same question.
const DEFAULT_PROVIDER: Provider = Provider::ClaudeCode;

/// What `ivar init` needs.
///
/// `name` and `provider` are plain, unvalidated strings — not yet
/// [`HallName`] / [`Provider`] — because `cli` cannot import `domain` (see
/// ARCHITECTURE.md's layering table) and therefore cannot validate them
/// itself. Validating them is this module's job; see [`init`].
#[derive(Debug, Clone)]
pub struct InitInput {
    /// Where to create the hall. A relative path resolves against
    /// [`Ctx::cwd`], never the process's real working directory.
    pub path: Utf8PathBuf,
    /// The hall's name, unvalidated. `None` derives it from the target
    /// directory's name — see [`init`]'s doc comment.
    pub name: Option<String>,
    /// The provider id, unvalidated. `None` defaults to
    /// [`DEFAULT_PROVIDER`].
    pub provider: Option<String>,
}

/// What `ivar init` did.
///
/// `Serialize`d as-is for `--json`; the human surface ([`Self::write_human`])
/// formats this same value — never a second, independently computed
/// summary. See ARCHITECTURE.md, "1. `action` is the unit, and it has one
/// output shape".
#[derive(Debug, Clone, Serialize)]
pub struct InitOutcome {
    /// The hall root, resolved to an absolute, symlink-free path.
    pub root: Utf8PathBuf,
    /// The hall's name, as recorded in `ivar.json`.
    pub name: HallName,
    /// The provider recorded as both the sole available and the default one.
    pub provider: Provider,
}

impl WriteHuman for InitOutcome {
    /// The human-readable rendering `bin/ivar.rs` writes when `--json` is
    /// absent. Takes a writer rather than printing directly — `println!` is
    /// denied crate-wide (`clippy::print_stdout`) precisely so every
    /// user-facing byte flows through a seam like this one, which a test can
    /// point at an in-memory buffer.
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Initialised hall `{}` at {} (provider: {})",
            self.name, self.root, self.provider
        )
    }
}

/// Create a hall at `input.path`: write `ivar.json` (via `store::manifest`),
/// create `.ivar/`, and write the hall's `.gitignore` lines (via
/// [`Layout::gitignore_lines`]). Nothing else — no git clone, no harness
/// config materialisation, no skill home. Those are `ivar sync`'s job (see
/// ARCHITECTURE.md's build order).
///
/// Four decisions this slice settles:
///
/// - **A hall already there.** If `input.path` (once resolved) already has
///   an `ivar.json`, this refuses with
///   [`Status::Blocked`](crate::error::Status::Blocked) and two fix actions:
///   inspect it (`ivar status`, safe) or remove `ivar.json` and `.ivar/`
///   first (unsafe — `ivar.json` is committed and team-shared; blowing it
///   away can discard a teammate's settings). It never overwrites silently.
/// - **Nesting.** [`Layout::discover`] answers "is this already inside a
///   hall?" for free. A hall found *above* the target is refused the same
///   way — nothing else in this codebase (worktrees, view dirs, the
///   `.gitignore` this action writes) has any notion of one hall living
///   inside another, so blocking it here is cheaper than discovering the
///   ambiguity later, deeper in the stack.
/// - **The name, undeclared.** Falls back to the target directory's final
///   path component. A directory called `.acme` still fails — [`HallName`]
///   rejects a leading `.` whether it came from `--name` or from the
///   filesystem — with an extra fix action pointing at `--name`, since
///   derivation is a convenience, not a second, looser set of rules.
/// - **The provider, undeclared.** Falls back to [`DEFAULT_PROVIDER`]
///   (`claude-code`). `providers.available` is recorded as exactly that one
///   provider; `ivar sync`, next slice, is what a second provider needs.
///
/// # A directory can be created even when this returns `Err`
///
/// [`fs::ensure_dir`] runs *before* name/provider validation, so a bad
/// `--name` on a target that did not exist yet can still leave behind an
/// empty directory. That is a deliberate, narrow exception to "blocked means
/// nothing happened": resolving "does a hall already exist here, or above
/// here?" requires the target to exist (to canonicalise it), the same way
/// `git init <dir>` creates its target unconditionally, before it does
/// anything else. The promise this action actually keeps is the one that
/// matters: no half-written `ivar.json`, `.ivar/`, or `.gitignore` — the
/// three things it is responsible for.
///
/// The `clippy::result_large_err` threshold is raised project-wide in
/// `clippy.toml` rather than allowed per function: `Failure` is large by
/// design (see `error.rs`, not owned by this
/// slice), and `Outcome<T> = Result<Report<T>, Failure>` is the seam this
/// slice is establishing for every future verb — boxing it here would mean
/// every verb after `init` boxes it too, for a lint about a type this
/// module did not choose the shape of.
pub fn init(ctx: &Ctx, input: InitInput) -> Outcome<InitOutcome> {
    let target = ctx.resolve(&input.path);
    fs::ensure_dir(&target)?;

    let root =
        target
            .canonicalize_utf8()
            .map_err(|source| layout::DiscoverError::Unresolvable {
                path: target.clone(),
                source,
            })?;

    if let Some(found) = Layout::discover(&root)? {
        return Err(if found.root() == root.as_path() {
            already_initialised(found.manifest())
        } else {
            nested_inside(&root, found.root())
        });
    }

    let layout = Layout::at(root.clone());
    let name = resolve_name(input.name.as_deref(), &root)?;
    let provider = resolve_provider(input.provider.as_deref())?;

    let manifest = Manifest::new(
        name.clone(),
        Providers::new(vec![provider], provider),
        Vec::new(),
        None,
    )?;

    Manifest::write(&layout, &manifest)?;
    create_ivar_dir(&layout)?;
    gitignore::ensure(&layout)?;

    Ok(Report::new(InitOutcome {
        root,
        name,
        provider,
    }))
}

/// A hall's `ivar.json` already sits at `manifest_path`. Blocked, with a
/// safe fix (inspect) ordered ahead of an unsafe one (remove and reinit) —
/// see [`init`]'s doc comment for why removal is marked unsafe.
fn already_initialised(manifest_path: Utf8PathBuf) -> Failure {
    Failure::blocked(
        "hall.already_initialised",
        format!("a hall already exists at `{manifest_path}`"),
    )
    .expected("no ivar.json in this directory yet")
    .actual(format!("`{manifest_path}` already exists"))
    .fix(
        FixAction::safe("hall.inspect_existing", "See this hall's current state.")
            .command("ivar status"),
    )
    .fix(FixAction::unsafe_(
        "hall.reinitialise",
        "Remove `ivar.json` and `.ivar/` first if you intend to replace this hall.",
    ))
}

/// `attempted` is inside the hall already rooted at `existing_root`. Blocked
/// — see [`init`]'s doc comment for why nesting has no defined behaviour to
/// fall back on.
fn nested_inside(attempted: &Utf8Path, existing_root: &Utf8Path) -> Failure {
    Failure::blocked(
        "hall.nested",
        format!("`{attempted}` is inside the existing hall at `{existing_root}`"),
    )
    .expected("a directory outside any existing hall")
    .actual(format!(
        "already inside the hall rooted at `{existing_root}`"
    ))
    .fix(FixAction::safe(
        "hall.choose_outside_directory",
        format!("Choose a directory outside the existing hall at `{existing_root}`."),
    ))
}

/// The hall's name: `raw` if given, otherwise derived from `root`'s final
/// path component. Either way it is validated as a [`HallName`] — a derived
/// name gets an extra fix action pointing at `--name`, since falling back to
/// the directory name is a convenience, not a looser rule.
fn resolve_name(raw: Option<&str>, root: &Utf8Path) -> Result<HallName, Failure> {
    match raw {
        Some(value) => HallName::new(value).map_err(Failure::from),
        None => {
            let derived = root.file_name().ok_or_else(|| {
                Failure::blocked(
                    "hall.name_required",
                    "cannot derive a hall name from the root directory",
                )
                .expected("a directory with a name, or an explicit --name")
                .actual(format!("`{root}` has no file-name component"))
                .fix(FixAction::safe(
                    "hall.pass_name",
                    "Pass --name to choose the hall's name explicitly.",
                ))
            })?;
            HallName::new(derived).map_err(|error| {
                let failure: Failure = error.into();
                failure.fix(FixAction::safe(
                    "hall.pass_name",
                    "Or pass --name to choose a hall name explicitly.",
                ))
            })
        }
    }
}

/// The provider: `raw` parsed if given, otherwise [`DEFAULT_PROVIDER`].
fn resolve_provider(raw: Option<&str>) -> Result<Provider, Failure> {
    match raw {
        Some(value) => value.parse::<Provider>().map_err(Failure::from),
        None => Ok(DEFAULT_PROVIDER),
    }
}

/// Create the hall's `.ivar/` dotdir, empty.
///
/// [`Layout`] exposes no direct accessor for the bare dotdir (only for
/// children under it). The path comes from [`Layout::ivar_dir`] — every path
/// under a hall is computed in `store::layout` and nowhere else, which is the
/// rule that made consolidating seven dotdirs into one a single-file change.
fn create_ivar_dir(layout: &Layout) -> Result<(), Failure> {
    fs::ensure_dir(&layout.ivar_dir())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `ivar status` — hall health, derived and rendered.
// ---------------------------------------------------------------------------

/// What `ivar status` found.
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// The hall's overall health.
    pub health: &'static str,
    /// One entry per repo, with its observed state.
    pub repos: Vec<RepoStatusEntry>,
}

/// One repo's observed state for the status report.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatusEntry {
    /// The repo's name.
    pub name: crate::domain::name::RepoName,
    /// Whether the bare clone exists.
    pub bare_cloned: bool,
    /// Whether the default worktree exists.
    pub worktree: bool,
}

impl WriteHuman for StatusOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Hall at {} — {}", self.root, self.health)?;
        for repo in &self.repos {
            let bare = if repo.bare_cloned {
                "cloned"
            } else {
                "missing"
            };
            let worktree = if repo.worktree {
                "worktree ok"
            } else {
                "no worktree"
            };
            writeln!(w, "  {}  {bare}  {worktree}", repo.name)?;
        }
        Ok(())
    }
}

/// Report the hall's health. Read-only — never mutates anything.
pub fn status(ctx: &Ctx) -> Outcome<StatusOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let mut repos = Vec::new();
    for repo in manifest.repos() {
        let bare = layout.repo_bare(repo.name());
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        let bare_state = git.target_state(&bare)?;
        let bare_cloned = matches!(bare_state, TargetState::Repository);
        let worktree_state = if bare_cloned {
            git.target_state(&worktree)?
        } else {
            TargetState::Absent
        };
        repos.push(RepoStatusEntry {
            name: repo.name().clone(),
            bare_cloned,
            worktree: matches!(worktree_state, TargetState::Repository),
        });
    }

    let health = Health::derive(
        &repos
            .iter()
            .map(|repo| RepoHealth {
                bare_cloned: repo.bare_cloned,
                default_worktree_present: Some(repo.worktree),
                ahead_of_bare: false,
            })
            .collect::<Vec<_>>(),
    );

    Ok(Report::new(StatusOutcome {
        root: layout.root().to_path_buf(),
        health: health_word(health),
        repos,
    }))
}

/// The one-word health label for the report. The ladder lives in
/// `domain::health`; this is only the rendering.
fn health_word(health: Health) -> &'static str {
    match health {
        Health::Uninitialized => "uninitialized",
        Health::Operational => "operational",
        Health::Stale => "stale",
        Health::Degraded => "degraded",
    }
}

/// The hall [`Ctx::cwd`] is inside, or a [`Failure`] saying there is none.
/// Shared with the other verbs that operate on the current hall.
fn discover_hall(ctx: &Ctx) -> Result<Layout, Failure> {
    super::discover_hall(ctx)
}

/// The manifest [`Layout::discover`] just proved exists.
fn read_manifest(layout: &Layout) -> Result<Manifest, Failure> {
    super::read_manifest(layout)
}

// ---------------------------------------------------------------------------
// `ivar doctor` — diagnose the hall and suggest fixes.
// ---------------------------------------------------------------------------

/// One diagnosed problem.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    /// A stable code for the problem, e.g. `repo.bare_missing`.
    pub code: &'static str,
    /// What is wrong, in one sentence.
    pub what: String,
    /// The suggested fix.
    pub fix: String,
}

/// What `ivar doctor` found.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorOutcome {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// Every diagnosed problem. Empty means a healthy hall.
    pub findings: Vec<Diagnosis>,
}

impl WriteHuman for DoctorOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.findings.is_empty() {
            writeln!(w, "No problems found in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Problems in {}:", self.root)?;
        for finding in &self.findings {
            writeln!(w, "  - {} — {}", finding.code, finding.what)?;
            writeln!(w, "    fix: {}", finding.fix)?;
        }
        Ok(())
    }
}

/// Diagnose the hall, read-only. `status` says *how healthy*; `doctor` says
/// *what is wrong and what to do about it*.
pub fn doctor(ctx: &Ctx) -> Outcome<DoctorOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let mut findings = Vec::new();
    for repo in manifest.repos() {
        let bare = layout.repo_bare(repo.name());
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        let bare_state = git.target_state(&bare)?;

        match bare_state {
            TargetState::Repository => {}
            TargetState::Occupied => findings.push(Diagnosis {
                code: "repo.bare_occupied",
                what: format!("`{}` exists but is not a git repository", bare),
                fix: "Remove it and run `ivar sync` to clone afresh.".to_owned(),
            }),
            TargetState::Absent => findings.push(Diagnosis {
                code: "repo.bare_missing",
                what: format!("`{}` has not been cloned", repo.name()),
                fix: "Run `ivar sync` to clone it.".to_owned(),
            }),
        }

        // The worktree question only has an answer when the clone is there.
        if bare_state == TargetState::Repository {
            match git.target_state(&worktree)? {
                TargetState::Repository => {}
                TargetState::Occupied => findings.push(Diagnosis {
                    code: "repo.worktree_occupied",
                    what: format!("`{}` exists but is not a git worktree", worktree),
                    fix: "Remove it and run `ivar sync` to materialise it afresh.".to_owned(),
                }),
                TargetState::Absent => findings.push(Diagnosis {
                    code: "repo.worktree_missing",
                    what: format!("`{}` has no default-branch worktree", repo.name()),
                    fix: "Run `ivar sync` to materialise it.".to_owned(),
                }),
            }
        }
    }

    Ok(Report::new(DoctorOutcome {
        root: layout.root().to_path_buf(),
        findings,
    }))
}

// ---------------------------------------------------------------------------
// `ivar migrate` — advance ivar.json's schema version, asking first.
// ---------------------------------------------------------------------------

/// The outcome for every plan that leaves the file exactly as it was — three of
/// the four, plus a declined prompt.
fn untouched(manifest: Utf8PathBuf, plan: MigrationPlan) -> MigrateOutcome {
    MigrateOutcome {
        manifest,
        plan,
        migrated: false,
    }
}

/// What `ivar migrate` found, and whether it acted.
#[derive(Debug, Clone, Serialize)]
pub struct MigrateOutcome {
    /// The file this is about. Always the hall's `ivar.json`.
    pub manifest: Utf8PathBuf,
    /// What migrating would do, decided before anything was written.
    pub plan: MigrationPlan,
    /// Whether the file was actually rewritten. `false` whenever the plan had
    /// nothing to do, and also when a human declined or nobody was asked.
    pub migrated: bool,
}

impl WriteHuman for MigrateOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match self.plan {
            MigrationPlan::Current { version } => {
                writeln!(
                    w,
                    "{} is at version {version}. Nothing to do.",
                    self.manifest
                )
            }
            MigrationPlan::Available { from, to } if self.migrated => {
                writeln!(w, "Migrated {} from version {from} to {to}.", self.manifest)?;
                writeln!(
                    w,
                    "Commit it: your teammates need this change before their ivar can write the file."
                )
            }
            MigrationPlan::Available { from, to } => {
                writeln!(
                    w,
                    "{} would migrate from version {from} to {to}.",
                    self.manifest
                )?;
                writeln!(w, "Nothing was written.")
            }
            MigrationPlan::Unreachable { from, to } => writeln!(
                w,
                "{} reports version {from}, and this build has no migration to reach version {to}.",
                self.manifest
            ),
            MigrationPlan::TooNew { found, highest } => writeln!(
                w,
                "{} is at version {found}; this build understands up to {highest}.",
                self.manifest
            ),
        }
    }
}

/// Advance `ivar.json`'s on-disk schema version, after showing what would
/// change and asking.
///
/// This verb exists because [`crate::store::versioned::Policy::Committed`]
/// refuses to advance a committed file on its own, and its refusal names this
/// command as the way forward. `ivar.json` is in git: rewriting it during
/// somebody's unrelated `ivar sync` would put a schema bump in their next
/// commit and break a teammate still on the older binary. So it is a team
/// event with a human behind it.
///
/// Interactive by the same rule as [`cleanup`], and for the same reason: on a
/// non-tty run it prints the plan and writes nothing, so a script or an agent
/// can never advance a committed file unattended. There is deliberately no
/// `--yes`.
///
/// Local state (`.ivar/state.json`, lockfiles) is not this verb's business —
/// it migrates itself silently on read, because nobody reviews it. See
/// `docs/reference/on-disk-format.md`.
pub fn migrate(ctx: &Ctx) -> Outcome<MigrateOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest_path = layout.manifest().to_path_buf();

    // `plan` deliberately, not `read_manifest`: a too-new or unreachable file
    // is exactly what this verb has to be able to describe, and reading it
    // would refuse before there was anything to say.
    let plan = Manifest::plan(&layout)?.ok_or_else(|| {
        Failure::blocked(
            "hall.manifest_vanished",
            format!("`{manifest_path}` disappeared while reading it"),
        )
        .expected("the ivar.json that was there a moment ago")
        .actual("it is gone")
        .fix(FixAction::safe("hall.retry", "Run the command again.").command("ivar migrate"))
    })?;

    // One destructure decides the whole verb: `Available` is the only plan with
    // work in it, and each of the other three reports differently.
    //
    // The two that describe an unusable hall carry a warning. Describing it is
    // the job; exiting 0 while doing it is not — nothing was refused and
    // nothing broke, so this is the warning channel (exit 1), not a `Failure`.
    let (from, to) = match plan {
        MigrationPlan::Available { from, to } => (from, to),
        MigrationPlan::Current { .. } => {
            return Ok(Report::new(untouched(manifest_path, plan)));
        }
        MigrationPlan::TooNew { found, highest } => {
            let warning = Warning::new(
                "hall.manifest_too_new",
                manifest_path.to_string(),
                format!(
                    "schema version {found}, but this build understands up to {highest} — upgrade ivar; this command cannot help"
                ),
            );
            return Ok(Report::with_warnings(
                untouched(manifest_path, plan),
                vec![warning],
            ));
        }
        MigrationPlan::Unreachable { from, to } => {
            let warning = Warning::new(
                "hall.manifest_unreachable",
                manifest_path.to_string(),
                format!(
                    "schema version {from} with no migration to version {to} — check this is the file you meant; a file at a version this format never had is not one ivar wrote"
                ),
            );
            return Ok(Report::with_warnings(
                untouched(manifest_path, plan),
                vec![warning],
            ));
        }
    };

    let question = format!("Migrate `{manifest_path}` from version {from} to {to}?");
    if !ask(
        &question,
        "migrate.write_prompt",
        "migrate.read_answer",
        Some(
            "This rewrites a committed file. Commit the result — a teammate on an older ivar will refuse it until they upgrade.",
        ),
    )? {
        return Ok(Report::new(untouched(manifest_path, plan)));
    }

    Manifest::migrate(&layout)?;

    Ok(Report::new(MigrateOutcome {
        manifest: manifest_path,
        plan,
        migrated: true,
    }))
}

// ---------------------------------------------------------------------------
// `ivar cleanup` — remove work left behind, asking before anything is deleted.
// ---------------------------------------------------------------------------

/// What `ivar cleanup` would remove, or removed.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupOutcome {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// Everything removed.
    pub removed: Vec<String>,
    /// Everything declined (the user said no, or the run was not a tty).
    pub kept: Vec<String>,
}

impl WriteHuman for CleanupOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.removed.is_empty() && self.kept.is_empty() {
            writeln!(w, "Nothing to clean up in {}.", self.root)?;
            return Ok(());
        }
        for path in &self.removed {
            writeln!(w, "  removed {path}")?;
        }
        for path in &self.kept {
            writeln!(w, "  kept    {path}")?;
        }
        Ok(())
    }
}

/// Remove stale state, asking before anything is deleted.
///
/// This is the one verb that can destroy work, so it is **interactive by
/// design** (ARCHITECTURE.md: `cleanup` is where removal lives, and it will
/// ask) and deliberately has no `--force` / `--dry-run` automation flags.
/// On a non-tty run it lists what *would* be removed and keeps everything —
/// a script can never delete through `ivar cleanup`.
///
/// What it removes today: bare clones of repos no longer in the manifest.
/// (Worktree removal is where uncommitted work lives; that stays a manual
/// `git worktree remove` until a later slice.)
pub fn cleanup(ctx: &Ctx) -> Outcome<CleanupOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let mut removed = Vec::new();
    let mut kept = Vec::new();

    let repos_dir = layout.repos_dir();
    if fs::is_dir(&repos_dir)? {
        for entry in fs::read_dir(&repos_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            // A repo still in the manifest is not stale.
            if manifest
                .repos()
                .iter()
                .any(|repo| repo.name().as_str() == name)
            {
                continue;
            }
            let repo_dir = repos_dir.join(name);
            if ask_remove(&repo_dir)? {
                fs::remove_path(&repo_dir)?;
                removed.push(repo_dir.to_string());
            } else {
                kept.push(repo_dir.to_string());
            }
        }
    }

    Ok(Report::new(CleanupOutcome {
        root: layout.root().to_path_buf(),
        removed,
        kept,
    }))
}

/// Ask before removing `path`. Returns `true` to remove, `false` to keep.
///
/// Non-tty runs answer `false` — cleanup must never delete without a human
/// looking at the question.
fn ask_remove(path: &Utf8Path) -> Result<bool, Failure> {
    ask(
        &format!("Remove `{path}`?"),
        "cleanup.write_prompt",
        "cleanup.read_answer",
        None,
    )
}

/// Ask `question` on stderr and read a yes/no from stdin. `true` only for an
/// explicit `y`.
///
/// **Non-tty runs answer `false` without asking.** That is the safety property
/// both callers depend on: neither a `cleanup` that deletes nor a `migrate`
/// that rewrites a committed file may act when there is nobody to read the
/// question. A pipe is not consent.
///
/// The prompt goes to stderr so that piping stdout — the machine surface —
/// never swallows the question, and `--json` output stays parseable.
///
/// `write_code` / `read_code` are the caller's own [`Failure::code`]s: these
/// are the stable identifiers a machine matches on, so each verb keeps its own
/// rather than inheriting a shared one from this helper.
fn ask(
    question: &str,
    write_code: &'static str,
    read_code: &'static str,
    caveat: Option<&str>,
) -> Result<bool, Failure> {
    if !crate::infra::term::is_tty(crate::infra::term::Stream::Stderr) {
        return Ok(false);
    }
    let mut stderr = io::stderr().lock();
    let write = |result: io::Result<()>| {
        result.map_err(|source| {
            Failure::failed(write_code, format!("could not write the prompt: {source}"))
        })
    };
    if let Some(caveat) = caveat {
        write(writeln!(stderr, "{caveat}"))?;
    }
    write(writeln!(stderr, "{question} [y/N] "))?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(|source| {
        Failure::failed(read_code, format!("could not read your answer: {source}"))
    })?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use camino::Utf8PathBuf;

    use super::*;
    use crate::error::Status;
    use crate::test_support::{hall_root, hall_root as utf8_temp_dir};

    fn fresh_input() -> InitInput {
        InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: None,
        }
    }

    // -----------------------------------------------------------------------
    // `ivar migrate`
    // -----------------------------------------------------------------------

    /// A hall whose `ivar.json` has been rewritten to `version`, returning the
    /// raw bytes now on disk so a test can prove they were left alone.
    fn hall_at_version(root: &Utf8Path, version: u32) -> String {
        let path = root.join("ivar.json");
        let text = fs::read_text(&path).unwrap().unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("version".to_owned(), serde_json::Value::from(version));
        fs::write_text(
            &path,
            &format!("{}\n", serde_json::to_string(&value).unwrap()),
        )
        .unwrap();
        fs::read_text(&path).unwrap().unwrap()
    }

    fn human(outcome: &MigrateOutcome) -> String {
        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn migrate_on_a_current_hall_reports_nothing_to_do_and_writes_nothing() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());
        init(&ctx, fresh_input()).unwrap();
        let before = fs::read_text(&root.join("ivar.json")).unwrap().unwrap();

        let report = migrate(&ctx).unwrap();

        assert_eq!(report.value.plan, MigrationPlan::Current { version: 1 });
        assert!(!report.value.migrated);
        assert!(report.is_clean());
        assert!(human(&report.value).contains("Nothing to do"));
        assert_eq!(
            fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
            before,
            "a no-op migrate must not rewrite the file"
        );
    }

    #[test]
    fn migrate_outside_a_hall_is_blocked() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root);

        let failure = migrate(&ctx).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn migrate_refuses_a_file_newer_than_this_build_without_touching_it() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());
        init(&ctx, fresh_input()).unwrap();
        let on_disk = hall_at_version(&root, 99);

        let report = migrate(&ctx).unwrap();

        assert_eq!(
            report.value.plan,
            MigrationPlan::TooNew {
                found: 99,
                highest: 1
            }
        );
        assert!(!report.value.migrated);
        // The whole point of `plan` over `read`: a too-new hall gets described,
        // not refused into silence.
        assert!(human(&report.value).contains("understands up to 1"));
        // ...but describing it must not report success. A warning is what
        // makes `bin/ivar.rs` exit 1 instead of 0.
        assert!(!report.is_clean(), "a too-new hall must not exit clean");
        assert_eq!(report.warnings[0].code, "hall.manifest_too_new");
        assert_eq!(
            fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
            on_disk,
            "a too-new file must never be modified"
        );
    }

    #[test]
    fn migrate_reports_an_unversioned_file_as_unreachable_rather_than_adopting_it() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());
        init(&ctx, fresh_input()).unwrap();
        let on_disk = hall_at_version(&root, 0);

        let report = migrate(&ctx).unwrap();

        // `ivar.json`'s chain is empty and its first public version is 1, so
        // there is no v0 to migrate from. Relabelling it as current would adopt
        // a foreign file as ours — the format contract forbids exactly that.
        assert_eq!(
            report.value.plan,
            MigrationPlan::Unreachable { from: 0, to: 1 }
        );
        assert!(!report.value.migrated);
        assert!(human(&report.value).contains("no migration to reach version 1"));
        assert!(
            !report.is_clean(),
            "an unreachable hall must not exit clean"
        );
        assert_eq!(report.warnings[0].code, "hall.manifest_unreachable");
        assert_eq!(
            fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
            on_disk
        );
    }

    #[test]
    fn a_non_tty_run_never_answers_yes() {
        // The safety property both `cleanup` and `migrate` rest on: with no
        // terminal there is nobody to read the question, and a pipe is not
        // consent. The test suite itself is the non-tty case.
        assert!(!ask("Delete everything?", "t.write", "t.read", None).unwrap());
        assert!(!ask("Rewrite it?", "t.write", "t.read", Some("careful")).unwrap());
    }

    #[test]
    fn init_creates_the_expected_on_disk_shape() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());

        let report = init(&ctx, fresh_input()).unwrap();

        assert!(report.is_clean());
        assert!(fs::is_file(&root.join("ivar.json")).unwrap());
        assert!(fs::is_dir(&root.join(".ivar")).unwrap());
        assert!(fs::is_file(&root.join(".gitignore")).unwrap());
        assert_eq!(report.value.root, root);
    }

    #[test]
    fn init_derives_the_name_from_the_directory_when_absent() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());

        let report = init(&ctx, fresh_input()).unwrap();

        let expected_name = root.file_name().unwrap();
        assert_eq!(report.value.name.as_str(), expected_name);
    }

    #[test]
    fn init_defaults_the_provider_to_claude_code() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root);

        let report = init(&ctx, fresh_input()).unwrap();

        assert_eq!(report.value.provider, Provider::ClaudeCode);
    }

    #[test]
    fn init_honours_an_explicit_name_and_provider() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root);

        let report = init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: Some("opencode".to_owned()),
            },
        )
        .unwrap();

        assert_eq!(report.value.name.as_str(), "acme");
        assert_eq!(report.value.provider, Provider::OpenCode);
    }

    #[test]
    fn init_rejects_a_second_init_in_the_same_directory() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root);

        init(&ctx, fresh_input()).unwrap();
        let error = init(&ctx, fresh_input()).unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "hall.already_initialised");
        assert!(!error.fix_actions.is_empty());
    }

    #[test]
    fn init_rejects_nesting_inside_an_existing_hall() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());
        init(&ctx, fresh_input()).unwrap();

        let nested = root.join("nested");
        fs::ensure_dir(&nested).unwrap();
        let error = init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("nested"),
                name: None,
                provider: None,
            },
        )
        .unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "hall.nested");
    }

    #[test]
    fn init_rejects_an_invalid_explicit_name() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root);

        let error = init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("../etc".to_owned()),
                provider: None,
            },
        )
        .unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "name.not_a_segment");
    }

    #[test]
    fn init_rejects_an_invalid_derived_name_with_an_extra_fix_action() {
        let (_guard, root) = utf8_temp_dir();
        let hidden = root.join(".hidden");
        fs::ensure_dir(&hidden).unwrap();
        let ctx = Ctx::new(hidden);

        let error = init(&ctx, fresh_input()).unwrap_err();

        assert_eq!(error.code, "name.hidden");
        assert!(
            error
                .fix_actions
                .iter()
                .any(|fix| fix.code == "hall.pass_name")
        );
    }

    #[test]
    fn init_rejects_an_invalid_provider_id() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root);

        let error = init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: None,
                provider: Some("claude".to_owned()),
            },
        )
        .unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "provider.unknown_id");
    }

    #[test]
    fn gitignore_uses_the_star_form_and_reincludes_committed_children() {
        let (_guard, root) = utf8_temp_dir();
        let ctx = Ctx::new(root.clone());

        init(&ctx, fresh_input()).unwrap();

        let content = fs::read_text(&root.join(".gitignore")).unwrap().unwrap();
        assert_eq!(
            content,
            ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n\
             .claude/commands/ivar-*.md\n.opencode/commands/ivar-*.md\n"
        );
        assert!(!content.lines().any(|line| line == ".ivar/"));
    }

    #[test]
    fn gitignore_preserves_existing_content_and_does_not_duplicate_on_rerun() {
        let (_guard, root) = utf8_temp_dir();
        fs::write_text(&root.join(".gitignore"), "node_modules/\n").unwrap();
        let ctx = Ctx::new(root.clone());

        init(&ctx, fresh_input()).unwrap();

        let content = fs::read_text(&root.join(".gitignore")).unwrap().unwrap();
        assert!(content.starts_with("node_modules/\n"));
        assert_eq!(content.matches(".ivar/*").count(), 1);
    }

    #[test]
    fn write_human_names_the_hall_root_and_provider() {
        let outcome = InitOutcome {
            root: Utf8PathBuf::from("/hall"),
            name: HallName::new("acme").unwrap(),
            provider: Provider::ClaudeCode,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Initialised hall `acme` at /hall (provider: claude-code)\n"
        );
    }

    // -- status ---------------------------------------------------------------

    fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();

        let origin = crate::test_support::seeded_repo(
            &root.parent().unwrap().join("origins").join("api"),
            "main",
        );
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![crate::store::manifest::Repo::new(
                crate::domain::name::RepoName::new("api").unwrap(),
                origin.as_str(),
                crate::domain::name::BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        (guard, root)
    }

    #[test]
    fn status_reports_a_fresh_hall_as_operational() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        init(&ctx, fresh_input()).unwrap();

        let report = status(&ctx).unwrap();

        assert_eq!(report.value.health, "operational");
        assert!(report.value.repos.is_empty());
    }

    #[test]
    fn status_reports_a_synced_hall_with_repos_as_operational() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = status(&ctx).unwrap();

        assert_eq!(report.value.health, "operational");
        assert_eq!(report.value.repos.len(), 1);
        assert!(report.value.repos[0].bare_cloned);
        assert!(report.value.repos[0].worktree);
    }

    #[test]
    fn status_reports_a_never_synced_repo_as_degraded() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root);

        let report = status(&ctx).unwrap();

        assert_eq!(report.value.health, "degraded");
        assert!(!report.value.repos[0].bare_cloned);
    }

    // -- doctor ---------------------------------------------------------------

    #[test]
    fn doctor_finds_nothing_in_a_healthy_hall() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = doctor(&ctx).unwrap();

        assert!(report.value.findings.is_empty());
    }

    #[test]
    fn doctor_names_a_missing_bare_clone_and_its_fix() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root);

        let report = doctor(&ctx).unwrap();

        assert_eq!(report.value.findings.len(), 1);
        assert_eq!(report.value.findings[0].code, "repo.bare_missing");
        assert!(report.value.findings[0].fix.contains("ivar sync"));
    }

    // -- cleanup --------------------------------------------------------------

    #[test]
    fn cleanup_in_a_non_tty_run_keeps_everything() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        // A repo dir for a repo that is no longer in the manifest.
        let stale = root.join(".ivar/repos/old");
        fs::ensure_dir(&stale).unwrap();

        let report = cleanup(&ctx).unwrap();

        // Non-tty: nothing is deleted without a human.
        assert!(report.value.removed.is_empty());
        assert_eq!(report.value.kept.len(), 1);
        assert!(fs::is_dir(&stale).unwrap());
    }

    #[test]
    fn cleanup_leaves_repos_still_in_the_manifest_alone() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = cleanup(&ctx).unwrap();

        assert!(report.value.removed.is_empty());
        assert!(report.value.kept.is_empty());
        assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    }

    #[test]
    fn the_human_surface_of_status_names_the_health() {
        let outcome = StatusOutcome {
            root: Utf8PathBuf::from("/hall"),
            health: "operational",
            repos: vec![RepoStatusEntry {
                name: crate::domain::name::RepoName::new("api").unwrap(),
                bare_cloned: true,
                worktree: true,
            }],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Hall at /hall — operational\n  api  cloned  worktree ok\n"
        );
    }
}
