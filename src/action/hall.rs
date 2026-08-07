//! `hall`: `init` (this slice). `sync · status · doctor · cleanup` land in
//! later slices — see ARCHITECTURE.md's build order.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::HallName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::layout::{self, Layout};
use crate::store::manifest::{Manifest, Providers};

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
    ensure_gitignore(&layout)?;

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

/// Write (or extend) `<root>/.gitignore` with [`Layout::gitignore_lines`].
///
/// Reads any existing content first and only appends lines not already
/// present, rather than overwriting: a hall is very often initialised inside
/// a directory that already has its own `.gitignore` (`node_modules/`, build
/// output, ...), and clobbering that would be its own silent-overwrite bug —
/// the same one [`init`]'s doc comment refuses to commit for `ivar.json`.
fn ensure_gitignore(layout: &Layout) -> Result<(), Failure> {
    let path = layout.gitignore_path();
    let mut content = fs::read_text(&path)?.unwrap_or_default();

    for line in Layout::gitignore_lines() {
        if !content.lines().any(|existing| existing == line) {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(line);
            content.push('\n');
        }
    }

    fs::write_atomic(&path, content.as_bytes())?;
    Ok(())
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
    use crate::test_support::hall_root as utf8_temp_dir;

    fn fresh_input() -> InitInput {
        InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: None,
        }
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
        assert_eq!(content, ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n");
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
}
