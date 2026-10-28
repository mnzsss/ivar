//! `ivar init` — create a hall: `ivar.json`, `.ivar/`, and the selected
//! provider's shipped workflow commands. See the parent module doc for the
//! full contract.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::infra::fs;
use crate::infra::proc;
use crate::store::gitignore;
use crate::store::layout::{self, Layout};
use crate::store::manifest::{Manifest, Providers};

use super::super::{Ctx, sync};

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
    /// Run `git init -b main` in the hall root when it is not already inside
    /// a git repository.
    pub git: bool,
    /// Add an `origin` remote with this URL. Implies [`Self::git`].
    pub origin: Option<String>,
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
    /// Whether the hall root is inside a git repository. Only probed when
    /// `--git` or `--origin` was given; `false` otherwise, whatever the
    /// directory really is.
    pub git: bool,
    /// The `origin` URL of that repository, when git was requested and the
    /// remote exists. `None` otherwise.
    pub origin: Option<String>,
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
        )?;
        match (self.git, &self.origin) {
            (true, Some(origin)) => writeln!(w, "Git repository ready (origin: {origin})"),
            (true, None) => writeln!(w, "Git repository ready (no origin)"),
            (false, _) => Ok(()),
        }
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
/// With `--git` / `--origin` the hall root also becomes a git repository (see
/// [`apply_git`]). That step runs after the manifest is durable and a git
/// failure is a warning, never a rollback. Re-running `ivar init` with either
/// flag on an existing hall skips the hall creation and completes only the git
/// part, so a failed attempt can be repaired by repeating the command. An
/// `origin` that already exists with a different URL is refused before
/// anything is written.
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
pub fn init(ctx: &Ctx, input: &InitInput) -> Outcome<InitOutcome> {
    let origin = validate_origin(input.origin.as_deref())?;
    let want_git = input.git || origin.is_some();
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
        if found.root() == root.as_path() && want_git {
            return complete_git(input, &found, origin.as_deref());
        }
        return Err(if found.root() == root.as_path() {
            already_initialised(&found.manifest())
        } else {
            nested_inside(&root, found.root())
        });
    }

    // Read-only: refuse a conflicting `origin` before anything is written.
    if let Some(url) = origin.as_deref() {
        check_origin_conflict(&root, url)?;
    }

    let layout = Layout::at(root.clone());
    let name = resolve_name(input.name.as_deref(), &root)?;
    let provider = resolve_provider(input.provider.as_deref())?;

    let manifest = Manifest::new(
        name.clone(),
        Providers::new(vec![provider], provider),
        Vec::new(),
    )?;

    Manifest::write(&layout, &manifest)?;
    create_ivar_dir(&layout)?;
    gitignore::ensure(&layout)?;

    // The manifest and skeleton are on disk before the convenience files are
    // attempted, and a failed attempt is a warning, never a failure: the hall
    // is valid and `ivar sync` repairs the rest.
    let mut report = Report::new(InitOutcome {
        root: root.clone(),
        name,
        provider,
        git: false,
        origin: None,
    });
    if want_git {
        apply_git(&mut report, &root, origin.as_deref());
    }
    // The canonical instructions and the first provider alias, through the
    // same reconciler `ivar sync` runs. A conflict (say, a pre-existing
    // regular CLAUDE.md) warns without rolling the hall back.
    for warning in sync::materialise_instructions(&layout, &manifest) {
        report.warn(warning);
    }
    if let Some(warning) = sync::materialise_commands(&layout, provider) {
        report.warn(warning);
    }
    if let Some(warning) = sync::materialise_shipped_skills(&layout, provider) {
        report.warn(warning);
    }
    Ok(report)
}

/// Re-run of `ivar init --git/--origin` on a hall that already exists: skip
/// the hall creation and complete only the git part, so the command is
/// idempotent and repairs a previous partial run.
fn complete_git(input: &InitInput, found: &Layout, origin: Option<&str>) -> Outcome<InitOutcome> {
    let manifest = super::super::read_manifest(found)?;
    let root = found.root().to_owned();
    if let Some(url) = origin {
        check_origin_conflict(&root, url)?;
    }
    let mut report = Report::new(InitOutcome {
        root: root.clone(),
        name: manifest.name().clone(),
        provider: manifest.providers().default_provider(),
        git: false,
        origin: None,
    });
    // Repeating the same command is the point, so only a value that differs
    // from the existing hall is worth a warning.
    let name_differs = input
        .name
        .as_deref()
        .is_some_and(|name| name != manifest.name().as_str());
    let provider_differs = input
        .provider
        .as_deref()
        .is_some_and(|raw| raw.parse::<Provider>().ok() != Some(report.value.provider));
    if name_differs || provider_differs {
        report.warn(Warning::new(
            "hall.init_options_ignored",
            root.as_str(),
            "the hall already exists; the differing --name/--provider were not applied",
        ));
    }
    apply_git(&mut report, &root, origin);
    Ok(report)
}

/// The `--origin` value, validated: non-empty, and not shaped like a git
/// option (`git remote add origin -x` would parse it as a flag).
fn validate_origin(raw: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let url = raw.trim();
    if url.is_empty() || url.starts_with('-') {
        return Err(Failure::blocked(
            "hall.origin_invalid",
            format!("`{raw}` is not a usable origin URL"),
        )
        .expected("a git remote URL or path, not empty and not starting with `-`")
        .actual(format!("`{raw}`"))
        .fix(FixAction::safe(
            "hall.pass_origin",
            "Pass the repository's URL, e.g. `--origin git@github.com:acme/hall.git`.",
        )));
    }
    Ok(Some(url.to_owned()))
}

/// Run `git` in `root`, returning its output, or the failure text.
fn run_git(root: &Utf8Path, args: &[&str]) -> Result<proc::Output, String> {
    proc::capture(
        &proc::Command::new("git")
            .args(args.iter().copied())
            .cwd(root),
    )
    .map_err(|error| error.to_string())
}

/// Whether `root` is inside a git work tree (its own, or an enclosing one).
fn inside_work_tree(root: &Utf8Path) -> bool {
    run_git(root, &["rev-parse", "--is-inside-work-tree"])
        .is_ok_and(|out| out.success() && out.stdout.trim() == "true")
}

/// The `origin` URL of the repository `root` is in, if it has one.
fn current_origin(root: &Utf8Path) -> Option<String> {
    run_git(root, &["remote", "get-url", "origin"])
        .ok()
        .filter(proc::Output::success)
        .map(|out| out.stdout.trim().to_owned())
        .filter(|url| !url.is_empty())
}

/// Blocked when `root` is already in a repository whose `origin` is a
/// different URL than `wanted`. Never overwrites; shows both URLs.
fn check_origin_conflict(root: &Utf8Path, wanted: &str) -> Result<(), Failure> {
    if !inside_work_tree(root) {
        return Ok(());
    }
    match current_origin(root) {
        Some(existing) if existing != wanted => Err(Failure::blocked(
            "hall.origin_conflict",
            format!("the hall's repository already has an `origin` ({existing})"),
        )
        .expected(format!("`origin` to be `{wanted}`, or absent"))
        .actual(format!("`origin` is `{existing}`"))
        .fix(FixAction::safe(
            "hall.keep_origin",
            "Run `ivar init` without `--origin`, or pass the URL `origin` already has.",
        ))
        .fix(FixAction::unsafe_(
            "hall.replace_origin",
            format!("Run `git remote set-url origin <url>` in `{root}` if you mean to change it."),
        ))),
        _ => Ok(()),
    }
}

/// Make the hall root a git repository with the requested `origin`, then
/// record what is true in the outcome. Failures are warnings: the manifest is
/// already durable, and repeating `ivar init --git/--origin` finishes the job.
fn apply_git(report: &mut Report<InitOutcome>, root: &Utf8Path, origin: Option<&str>) {
    let warn = |report: &mut Report<InitOutcome>, what: String| {
        report.warn(Warning::new("hall.git_failed", root.as_str(), what));
    };

    if !inside_work_tree(root) {
        match run_git(root, &["init", "-q", "-b", "main"]) {
            Ok(out) if out.success() => {}
            Ok(out) => warn(report, format!("`git init` failed: {}", out.diagnostic())),
            Err(error) => warn(report, format!("could not run `git init`: {error}")),
        }
    }

    report.value.git = inside_work_tree(root);
    if !report.value.git {
        return;
    }

    if let Some(url) = origin
        && current_origin(root).is_none()
    {
        match run_git(root, &["remote", "add", "origin", url]) {
            Ok(out) if out.success() => {}
            Ok(out) => warn(
                report,
                format!("`git remote add origin` failed: {}", out.diagnostic()),
            ),
            Err(error) => warn(
                report,
                format!("could not run `git remote add origin`: {error}"),
            ),
        }
    }
    report.value.origin = current_origin(root);
}

/// A hall's `ivar.json` already sits at `manifest_path`. Blocked, with a
/// safe fix (inspect) ordered ahead of an unsafe one (remove and reinit) —
/// see [`init`]'s doc comment for why removal is marked unsafe.
fn already_initialised(manifest_path: &Utf8PathBuf) -> Failure {
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
        "already inside the hall rooted at `{existing_root}`; every ivar command run here would act on that hall"
    ))
    .fix(
        FixAction::safe(
            "hall.choose_outside_directory",
            format!(
                "Create the new hall outside `{existing_root}`, for a throwaway one a fresh temp dir."
            ),
        )
        .command("cd \"$(mktemp -d)\" && ivar init"),
    )
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
