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
use crate::harness::commands::{self, Inspection, Integrity};
use crate::infra::fs;
use crate::store::gitignore;
use crate::store::layout::{self, Layout};
use crate::store::manifest::{Manifest, MigrationPlan, Providers};

use super::Ctx;
use super::sync;

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
/// create `.ivar/`, write the hall's `.gitignore` lines (via
/// [`Layout::gitignore_lines`]), and materialise the selected provider's
/// shipped workflow commands into its native command directory. Nothing else —
/// no git clone, no harness config materialisation, no skill home. Those are
/// `ivar sync`'s job (see ARCHITECTURE.md's build order).
///
/// The commands are attempted only after the manifest and skeleton are
/// durable, and their failure is a **warning**, not a rollback: a hall that
/// cannot materialise its commands is still a valid hall, and `ivar sync` is
/// the repair. The manifest never loses to a convenience file.
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

    // The manifest and skeleton are on disk before the commands are attempted,
    // and a failed attempt is a warning, never a failure: the hall is valid
    // and `ivar sync` repairs the commands.
    let mut report = Report::new(InitOutcome {
        root,
        name,
        provider,
    });
    if let Err(warning) = sync::materialise_commands(&layout, provider) {
        report.warn(warning);
    }
    Ok(report)
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

    // Shipped workflow commands: a missing or modified official command is a
    // problem worth naming, but a *convenience* one — it never changes hall
    // health and never blocks session start. `ivar sync` is the repair.
    for provider in Provider::ALL {
        let enabled = manifest.providers().available().contains(&provider);
        let dir = layout.commands_dir(&provider);
        match commands::inspect(&dir, enabled) {
            Ok(inspections) => {
                for inspection in inspections {
                    if let Some(diagnosis) = command_diagnosis(provider, &inspection, enabled) {
                        findings.push(diagnosis);
                    }
                }
            }
            Err(error) => findings.push(Diagnosis {
                code: "provider.commands_inspect_failed",
                what: format!("could not inspect {provider}'s workflow commands: {error}"),
                fix: "Run `ivar sync` to reconcile them.".to_owned(),
            }),
        }
    }

    Ok(Report::new(DoctorOutcome {
        root: layout.root().to_path_buf(),
        findings,
    }))
}

/// One diagnosis for a command file that is not in its target state.
///
/// Every finding says `ivar sync` repairs it — it does — except
/// `legacy_command_modified`, where sync preserves the customized file by
/// design, so the way out is the user reviewing it and renaming or removing
/// it themselves.
fn command_diagnosis(
    provider: Provider,
    inspection: &Inspection,
    enabled: bool,
) -> Option<Diagnosis> {
    let file_name = inspection.path.file_name().unwrap_or("file");
    match inspection.integrity {
        Integrity::Current => None,
        Integrity::Missing => Some(Diagnosis {
            code: "provider.command_missing",
            what: format!(
                "{provider}'s `/ivar-{}` command is missing (`{file_name}`)",
                inspection.id
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        Integrity::Modified => Some(Diagnosis {
            code: "provider.command_modified",
            what: format!(
                "{provider}'s `/ivar-{}` command has been modified (`{file_name}`)",
                inspection.id
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        Integrity::LegacyModified => Some(Diagnosis {
            code: "provider.legacy_command_modified",
            what: format!(
                "{provider}'s legacy `{file_name}` command was customised and is preserved"
            ),
            fix: "Review it, then rename or remove it — `ivar sync` keeps it by design.".to_owned(),
        }),
        Integrity::Stale => Some(Diagnosis {
            code: "provider.command_stale",
            what: if enabled {
                format!("{provider}'s `{file_name}` is not an ivar-shipped command")
            } else {
                format!("{provider} is no longer listed, but its `{file_name}` command remains")
            },
            fix: "Run `ivar sync` to remove it.".to_owned(),
        }),
    }
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
#[path = "../../tests/unit/action/hall.rs"]
mod tests;
