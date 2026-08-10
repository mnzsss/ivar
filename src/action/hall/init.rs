//! `ivar init` — create a hall: `ivar.json`, `.ivar/`, and the selected
//! provider's shipped workflow commands. See the parent module doc for the
//! full contract.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
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
