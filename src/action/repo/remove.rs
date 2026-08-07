//! `ivar repo remove` — drop a repo from `ivar.json`.
//!
//! Removing never touches the filesystem. The repo's bare clone and worktrees
//! stay under `.ivar/` until `ivar cleanup` (slice 8) is asked to remove them,
//! because deleting a worktree can destroy uncommitted work and that is a
//! decision a removal-from-config command does not get to make on its own.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::name::RepoName;
use crate::error::{Outcome, Report, WriteHuman};

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;
use crate::store::manifest::Manifest;

/// What `ivar repo remove` needs.
#[derive(Debug, Clone)]
pub struct RemoveInput {
    /// The repo's name, unvalidated — [`RepoName`] is this module's job.
    pub name: String,
}

/// What `ivar repo remove` did.
#[derive(Debug, Clone, Serialize)]
pub struct RemoveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repo, as it was removed from `ivar.json`.
    pub name: RepoName,
}

impl WriteHuman for RemoveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Removed repo `{}` from ivar.json in {}. Its files stay on disk — run `ivar cleanup` to remove them.",
            self.name, self.root,
        )
    }
}

/// Remove `input.name` from `ivar.json`.
///
/// A repo that is not in the manifest is blocked ([`Manifest::with_repo_removed`]
/// refuses it with `repo.not_found`), so a typo cannot silently "succeed".
pub fn remove(ctx: &Ctx, input: RemoveInput) -> Outcome<RemoveOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let name = RepoName::new(input.name)?;
    let updated = manifest.with_repo_removed(&name)?;
    Manifest::write(&layout, &updated)?;

    Ok(Report::new(RemoveOutcome {
        root: layout.root().to_path_buf(),
        name,
    }))
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
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
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
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        (guard, root)
    }

    #[test]
    fn remove_drops_the_repo_from_ivar_json_and_keeps_its_files() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        // Materialise the repo first, so there is something on disk to keep.
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let report = remove(&ctx, RemoveInput { name: "api".to_owned() }).unwrap();

        assert!(report.is_clean());
        let layout = Layout::at(root.clone());
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        assert!(manifest.repos().is_empty());
        // The clone stays — removal is config-only.
        assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    }

    #[test]
    fn remove_rejects_a_repo_that_is_not_in_the_manifest() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root);

        let failure = remove(&ctx, RemoveInput { name: "ghost".to_owned() }).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "manifest.repo_not_found");
    }

    #[test]
    fn remove_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = remove(&ctx, RemoveInput { name: "api".to_owned() }).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn the_human_surface_says_removal_is_config_only() {
        let outcome = RemoveOutcome {
            root: Utf8PathBuf::from("/hall"),
            name: RepoName::new("api").unwrap(),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Removed repo `api` from ivar.json in /hall. Its files stay on disk — run `ivar cleanup` to remove them.\n"
        );
    }
}
