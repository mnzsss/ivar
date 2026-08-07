//! `ivar repo add` — declare a repo in `ivar.json`, clone it bare, and
//! materialise its default-branch worktree.
//!
//! # The three collisions
//!
//! - **Name already declared** — blocked. `ivar.json` is committed and
//!   team-shared; two entries with one name would be an invariant violation
//!   (`Manifest::validate` refuses it on read too), so the command refuses
//!   before writing anything.
//! - **URL already tracked under another name** — blocked. Two names for one
//!   remote is how a hall ends up with two bare clones of the same repo, and
//!   the fix action names the existing entry rather than guessing.
//! - **Bare clone already on disk** — the user chooses. `--reuse` keeps it,
//!   `--fresh` deletes and re-clones; with neither flag the command is
//!   [`Status::Blocked`](crate::error::Status::Blocked) with fix actions
//!   naming both, so an agent in `--json` mode can recover unattended with
//!   the safe one.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::{BranchName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::fs;
use crate::store::manifest::{Manifest, Repo};

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar repo add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    /// The repo's name, unvalidated — [`RepoName`] is this module's job.
    pub name: String,
    /// The git remote URL. Never validated as a path; it is a remote, not a
    /// directory.
    pub url: String,
    /// The default branch, unvalidated. `None` defaults to `main`.
    pub default_branch: Option<String>,
    /// How to treat a bare clone that already exists: `Some(true)` reuses it,
    /// `Some(false)` deletes and re-clones, `None` blocks and asks.
    pub reuse_existing: Option<bool>,
}

/// What `ivar repo add` did.
#[derive(Debug, Clone, Serialize)]
pub struct AddOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repo, as now recorded in `ivar.json`.
    pub name: RepoName,
    /// The git remote URL.
    pub url: String,
    /// The branch a fresh worktree defaults to.
    pub default_branch: BranchName,
    /// Whether an existing bare clone was reused rather than re-cloned.
    pub bare_clone_reused: bool,
}

impl WriteHuman for AddOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let reused = if self.bare_clone_reused {
            " (reused existing clone)"
        } else {
            ""
        };
        writeln!(
            w,
            "Added repo `{}` at {} ← {}{reused}",
            self.name, self.root, self.url,
        )
    }
}

/// Add `input.name`/`input.url` to `ivar.json`, clone it bare, and create
/// the default-branch worktree.
///
/// The manifest is rewritten **after** the clone lands, so a repo that fails
/// to clone never leaves a half-declared entry behind — `ivar.json` is
/// committed, and a declaration that points at nothing on disk is a promise
/// to every teammate who runs `ivar sync`.
pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<AddOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let name = RepoName::new(input.name)?;
    let default_branch = match input.default_branch.as_deref() {
        Some(raw) => BranchName::new(raw)?,
        None => BranchName::new("main").map_err(|_| {
            Failure::blocked(
                "repo.add_needs_branch",
                "cannot default to `main`: `main` is not a valid branch name",
            )
            .expected("a valid default branch")
            .actual("`main` was refused by git's branch-name rules")
            .fix(FixAction::safe(
                "repo.pass_branch",
                "Pass --default-branch with a branch name git accepts.",
            ))
        })?,
    };

    // Collision 1: the name must be free.
    for existing in manifest.repos() {
        if existing.name() == &name {
            return Err(Failure::blocked(
                "repo.name_exists",
                format!("`{name}` is already in ivar.json"),
            )
            .expected("a repo name not already declared")
            .actual(format!("`{name}` is already declared"))
            .fix(FixAction::safe(
                "repo.remove_first",
                format!("Remove `{name}` first with `ivar repo remove {name}`, then add again."),
            )));
        }
    }

    // Collision 2: the URL must not already be tracked under another name.
    if let Some(existing) = manifest.repos().iter().find(|r| r.url() == input.url) {
        return Err(Failure::blocked(
            "repo.url_exists",
            format!(
                "`{}` is already tracked as `{}`",
                input.url,
                existing.name()
            ),
        )
        .expected("a URL not already in the manifest")
        .actual(format!("`{}` already points at this URL", existing.name()))
        .fix(FixAction::safe(
            "repo.use_existing",
            format!("Use the existing entry `{}` instead.", existing.name()),
        )));
    }

    let bare = layout.repo_bare(&name);
    let worktree = layout.repo_worktree(&name, &default_branch);

    // Collision 3: a bare clone already on disk — reuse, replace, or block.
    let bare_clone_reused = match git.target_state(&bare)? {
        TargetState::Repository => match input.reuse_existing {
            Some(true) => true,
            Some(false) => {
                // `--fresh` means "clone it anew": the bare clone goes, and
                // with it the worktree that pointed at it — a worktree whose
                // gitdir names a removed repository is an orphan that no
                // `target_state` probe can recognise.
                fs::remove_path(&bare)?;
                fs::remove_path(&worktree)?;
                ensure_bare(&git, &input.url, &bare)?;
                false
            }
            None => {
                return Err(bare_exists_ask(&bare));
            }
        },
        TargetState::Occupied => {
            return Err(Failure::blocked(
                "repo.bare_occupied",
                format!("`{bare}` exists but is not a git repository"),
            )
            .expected("a bare clone, or nothing at all")
            .actual("a directory git does not recognise")
            .fix(FixAction::unsafe_(
                "repo.clear_bare",
                format!("Remove `{bare}` and run `ivar repo add` again."),
            )));
        }
        TargetState::Absent => {
            ensure_bare(&git, &input.url, &bare)?;
            false
        }
    };

    ensure_worktree(&git, &bare, &worktree, &default_branch)?;

    // The clone landed — now the declaration. Both in this order, so a failed
    // clone never leaves a dangling manifest entry.
    let updated = manifest.with_repo_added(Repo::new(
        name.clone(),
        input.url.clone(),
        default_branch.clone(),
    ))?;
    Manifest::write(&layout, &updated)?;

    Ok(Report::new(AddOutcome {
        root: layout.root().to_path_buf(),
        name,
        url: input.url,
        default_branch,
        bare_clone_reused,
    }))
}

/// A bare clone already sits at `path` and the caller gave no direction.
/// Blocked with the two ways out, safe first so an agent can pick `--reuse`.
fn bare_exists_ask(path: &camino::Utf8Path) -> Failure {
    Failure::blocked(
        "repo.bare_exists",
        format!("a bare clone already exists at `{path}`"),
    )
    .expected("no existing clone, or an explicit --reuse/--fresh")
    .actual(format!("`{path}` already holds a bare clone"))
    .fix(FixAction::safe(
        "repo.reuse",
        format!("Pass `--reuse` to keep the existing clone at `{path}`."),
    ))
    .fix(FixAction::unsafe_(
        "repo.fresh",
        format!("Pass `--fresh` to delete `{path}` and clone it anew."),
    ))
}

/// Clone `url` into `bare` as a bare repository, creating parents first.
fn ensure_bare(git: &impl git::Git, url: &str, bare: &camino::Utf8Path) -> Result<(), Failure> {
    if let Some(parent) = bare.parent() {
        fs::ensure_dir(parent)?;
    }
    git.clone_bare(url, bare)?;
    Ok(())
}

/// Add the default-branch worktree, creating parents first.
///
/// Idempotent: an existing worktree at `worktree` is left alone, because
/// `add` can legitimately run against a repo whose worktree a previous
/// (since-removed) declaration already materialised.
fn ensure_worktree(
    git: &impl git::Git,
    bare: &camino::Utf8Path,
    worktree: &camino::Utf8Path,
    branch: &BranchName,
) -> Result<(), Failure> {
    match git.target_state(worktree)? {
        TargetState::Repository => Ok(()),
        TargetState::Occupied => Err(Failure::blocked(
            "repo.worktree_occupied",
            format!("`{worktree}` exists but is not a git worktree"),
        )
        .expected("a worktree, or nothing at all")
        .actual("a directory git does not recognise")
        .fix(FixAction::unsafe_(
            "repo.clear_worktree",
            format!("Remove `{worktree}` and run `ivar repo add` again."),
        ))),
        TargetState::Absent => {
            if let Some(parent) = worktree.parent() {
                fs::ensure_dir(parent)?;
            }
            git.add_worktree(bare, worktree, branch.as_str())?;
            Ok(())
        }
    }
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
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::test_support::{hall_root, seeded_repo};

    fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf, String) {
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
        let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
        (guard, root, origin.as_str().to_owned())
    }

    fn input(url: &str) -> AddInput {
        AddInput {
            name: "api".to_owned(),
            url: url.to_owned(),
            default_branch: None,
            reuse_existing: None,
        }
    }

    #[test]
    fn add_clones_the_repo_and_declares_it_in_ivar_json() {
        let (_guard, root, url) = seeded_hall();
        let ctx = Ctx::new(root.clone());

        let report = add(&ctx, input(&url)).unwrap();

        assert!(report.is_clean());
        assert!(!report.value.bare_clone_reused);
        assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join(".ivar/repos/api/main/README.md")).unwrap(),
            "seed\n"
        );

        let layout = Layout::at(root);
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        assert_eq!(manifest.repos().len(), 1);
        assert_eq!(manifest.repos()[0].name().as_str(), "api");
    }

    #[test]
    fn add_is_idempotent_on_the_worktree_path_but_rejects_a_duplicate_name() {
        let (_guard, root, url) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        add(&ctx, input(&url)).unwrap();

        let error = add(&ctx, input(&url)).unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "repo.name_exists");
    }

    #[test]
    fn add_rejects_a_url_already_tracked_under_another_name() {
        let (_guard, root, url) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        add(&ctx, input(&url)).unwrap();

        let error = add(
            &ctx,
            AddInput {
                name: "api-2".to_owned(),
                ..input(&url)
            },
        )
        .unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "repo.url_exists");
        assert!(
            error.what.contains("api"),
            "the existing name must be named: {}",
            error.what
        );
    }

    /// The bare-exists collision fires when the name and URL are both free
    /// but a previous `add` (since removed from the manifest) left a clone
    /// on disk. The fix actions must name both ways out.
    #[test]
    fn add_blocks_when_a_bare_clone_exists_without_reuse_or_fresh() {
        let (_guard, root, url) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        add(&ctx, input(&url)).unwrap();
        // Remove the manifest entry; the bare clone stays on disk.
        let layout = Layout::at(root.clone());
        let mut manifest = Manifest::read(&layout).unwrap().unwrap();
        manifest = manifest
            .with_repo_removed(&RepoName::new("api").unwrap())
            .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        let error = add(&ctx, input(&url)).unwrap_err();

        assert_eq!(error.status, Status::Blocked);
        assert_eq!(error.code, "repo.bare_exists");
        assert!(
            error
                .fix_actions
                .iter()
                .any(|fix| fix.code == "repo.reuse" && fix.safe)
        );
        assert!(
            error
                .fix_actions
                .iter()
                .any(|fix| fix.code == "repo.fresh" && !fix.safe)
        );
    }

    #[test]
    fn reuse_keeps_the_existing_bare_clone() {
        let (_guard, root, url) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        add(&ctx, input(&url)).unwrap();
        let layout = Layout::at(root.clone());
        let mut manifest = Manifest::read(&layout).unwrap().unwrap();
        manifest = manifest
            .with_repo_removed(&RepoName::new("api").unwrap())
            .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        let report = add(
            &ctx,
            AddInput {
                name: "api".to_owned(),
                url,
                default_branch: None,
                reuse_existing: Some(true),
            },
        )
        .unwrap();

        assert!(report.value.bare_clone_reused);
        assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    }

    #[test]
    fn fresh_replaces_the_existing_bare_clone() {
        let (_guard, root, url) = seeded_hall();
        let ctx = Ctx::new(root.clone());
        add(&ctx, input(&url)).unwrap();
        let layout = Layout::at(root.clone());
        let mut manifest = Manifest::read(&layout).unwrap().unwrap();
        manifest = manifest
            .with_repo_removed(&RepoName::new("api").unwrap())
            .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        let report = add(
            &ctx,
            AddInput {
                name: "api".to_owned(),
                url,
                default_branch: None,
                reuse_existing: Some(false),
            },
        )
        .unwrap();

        assert!(!report.value.bare_clone_reused);
        assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    }

    #[test]
    fn add_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = add(&ctx, input("git@example.com:acme/api.git")).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn add_rejects_an_invalid_name() {
        let (_guard, root) = hall_root();
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

        let failure = add(
            &ctx,
            AddInput {
                name: "../etc".to_owned(),
                url: "git@example.com:acme/api.git".to_owned(),
                default_branch: None,
                reuse_existing: None,
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "name.not_a_segment");
    }

    #[test]
    fn the_human_surface_names_the_repo_and_whether_it_was_reused() {
        let outcome = AddOutcome {
            root: Utf8PathBuf::from("/hall"),
            name: RepoName::new("api").unwrap(),
            url: "git@example.com:acme/api.git".to_owned(),
            default_branch: BranchName::new("main").unwrap(),
            bare_clone_reused: true,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Added repo `api` at /hall ← git@example.com:acme/api.git (reused existing clone)\n"
        );
    }
}
