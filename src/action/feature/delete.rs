//! `ivar feature delete <name>` — tear a feature down, files and all.
//!
//! Delete is the destructive opposite of `close`: it removes the feature's
//! promoted repos' worktrees, its directory under `.ivar/features/` (promotion
//! record included), and its `plans/<name>/` directory.
//!
//! # Preflight, then mutate — never the other way around
//!
//! The whole feature tree is checked for removability before anything is
//! touched. Every blocking path is collected (path, why, mode, uid, gid) and
//! reported as one [`Failure::blocked`] with the full list in `details` — a
//! partial teardown is worse than none, because it strands worktrees with no
//! record pointing at them.
//!
//! # Best-effort teardown, and what that preserves
//!
//! Once the preflight passes, each worktree removal is best-effort: a git
//! refusal is a [`Warning`], never an abort of the batch. But a failed
//! worktree removal **preserves the feature record**: `feature.json` (and the
//! plans) stay on disk so the command can simply be re-run. Only when every
//! worktree is gone does the teardown proceed — plans first, feature directory
//! last — so `feature.json`, the record a retry needs, is the final thing to
//! disappear.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::discover_hall;
use crate::action::Ctx;

/// What `ivar feature delete` needs.
#[derive(Debug, Clone)]
pub struct DeleteInput {
    /// The feature's name.
    pub name: String,
}

/// One promoted repo's worktree removal.
#[derive(Debug, Clone, Serialize)]
pub struct WorktreeRemoval {
    /// The repo whose worktree was (or was not) removed.
    pub repo: RepoName,
    /// Whether the worktree is gone.
    pub removed: bool,
    /// Why it was not removed, when it was not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What `ivar feature delete` did.
#[derive(Debug, Clone, Serialize)]
pub struct DeleteOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature that was deleted.
    pub name: FeatureName,
    /// One entry per promoted repo, in name order.
    pub worktrees: Vec<WorktreeRemoval>,
    /// Whether the feature's own directory (and its record) was removed.
    pub feature_removed: bool,
    /// Whether the plans directory was removed.
    pub plans_removed: bool,
}

impl WriteHuman for DeleteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.feature_removed {
            writeln!(w, "Deleted feature `{}` in {}", self.name, self.root)
        } else {
            writeln!(
                w,
                "Deleted feature `{}` in {} — partially; the feature record is kept so the command can be retried",
                self.name, self.root
            )?;
            for removal in &self.worktrees {
                if !removal.removed {
                    writeln!(w, "  {}: worktree not removed", removal.repo)?;
                }
            }
            Ok(())
        }
    }
}

/// Why one path in the feature tree cannot be removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteBlocker {
    /// The path that cannot be removed.
    pub path: Utf8PathBuf,
    /// Why — a permission description, or the underlying I/O error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The path's permission bits, when they could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// The path's owning uid, when it could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    /// The path's owning gid, when it could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
}

/// Delete `input.name` and everything it owns.
///
/// Blocked when the feature does not exist, and when any path under its
/// directory cannot be removed — with every blocker collected before anything
/// is mutated. Failed when a teardown step dies mid-flight; the feature record
/// is preserved in that case so the command can be retried.
pub fn delete(ctx: &Ctx, input: DeleteInput) -> Outcome<DeleteOutcome> {
    let layout = discover_hall(ctx)?;
    let git = git::System;
    let name = FeatureName::new(input.name)?;

    let feature = Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })?;

    // Preflight: every path under the feature directory must be removable.
    // Nothing is mutated while any blocker stands.
    let blockers = collect_blockers(&layout.feature_dir(&name));
    if !blockers.is_empty() {
        let details = serde_json::to_value(&blockers).unwrap_or(serde_json::Value::Null);
        return Err(Failure::blocked(
            "feature.delete_blocked",
            format!(
                "cannot delete feature `{name}`: {} path(s) under its directory are not removable",
                blockers.len()
            ),
        )
        .expected("every path under the feature directory to be writable and searchable")
        .actual(format!(
            "{} path(s) could not be removed — see details for paths, modes, and owners",
            blockers.len()
        ))
        .fix(FixAction::safe(
            "feature.fix_permissions",
            format!(
                "Fix the permissions named above, then run `ivar feature delete {name}` again."
            ),
        ))
        .details(details));
    }

    // Teardown, worktree by worktree, best-effort.
    let mut warnings = Vec::new();
    let mut worktrees = Vec::new();
    let mut all_worktrees_removed = true;
    for repo in feature.promotions.keys() {
        let worktree = layout.repo_worktree(repo, &feature.branch);
        if !fs::is_dir(&worktree)? {
            // Nothing materialised — nothing to remove.
            worktrees.push(WorktreeRemoval {
                repo: repo.clone(),
                removed: true,
                detail: None,
            });
            continue;
        }
        match git.remove_worktree(&layout.repo_bare(repo), &worktree) {
            Ok(()) => worktrees.push(WorktreeRemoval {
                repo: repo.clone(),
                removed: true,
                detail: None,
            }),
            Err(error) => {
                all_worktrees_removed = false;
                let detail = error.to_string();
                warnings.push(Warning::new(
                    "feature.delete_worktree_failed",
                    repo.as_str(),
                    detail.clone(),
                ));
                worktrees.push(WorktreeRemoval {
                    repo: repo.clone(),
                    removed: false,
                    detail: Some(detail),
                });
            }
        }
    }

    if !all_worktrees_removed {
        // Keep the record and the plans: a retry must know which worktrees to
        // finish removing, and `feature.json` is that memory.
        return Ok(Report::with_warnings(
            DeleteOutcome {
                root: layout.root().to_path_buf(),
                name,
                worktrees,
                feature_removed: false,
                plans_removed: false,
            },
            warnings,
        ));
    }

    // Everything that can fail cheaply is gone. Plans go before the feature
    // directory: the record a retry needs is the last thing to disappear.
    fs::remove_path(&layout.plan_dir(&name)).map_err(|source| {
        Failure::failed(
            "feature.delete_plans_failed",
            format!("could not remove plans for feature `{name}`: {source}"),
        )
    })?;
    fs::remove_path(&layout.feature_dir(&name)).map_err(|source| {
        Failure::failed(
            "feature.delete_dir_failed",
            format!("could not remove feature `{name}`: {source}"),
        )
    })?;

    Ok(Report::with_warnings(
        DeleteOutcome {
            root: layout.root().to_path_buf(),
            name,
            worktrees,
            feature_removed: true,
            plans_removed: true,
        },
        warnings,
    ))
}

/// Walk `root` and report every path that cannot be removed.
///
/// A path is removable when it is writable and, if a directory, searchable —
/// checked against the mode bits directly (rather than an `access(2)` probe),
/// which is what lets the check report *why* with the mode, uid, and gid, and
/// what keeps it honest when the process runs as root, where permission
/// checks always answer yes.
fn collect_blockers(root: &Utf8Path) -> Vec<DeleteBlocker> {
    use std::os::unix::fs::MetadataExt as _;

    let mut blockers = Vec::new();
    for entry in walkdir::WalkDir::new(root.as_std_path()) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => {
                blockers.push(DeleteBlocker {
                    path: root.to_path_buf(),
                    error: Some(source.to_string()),
                    mode: None,
                    uid: None,
                    gid: None,
                });
                continue;
            }
        };
        let std_path = entry.path().to_path_buf();
        let path = Utf8PathBuf::from_path_buf(std_path.clone())
            .unwrap_or_else(|raw| Utf8PathBuf::from(raw.to_string_lossy().into_owned()));

        match fs_err::symlink_metadata(&std_path) {
            Ok(metadata) => {
                let mode = metadata.mode();
                let writable = mode & 0o222 != 0;
                let searchable = !metadata.is_dir() || mode & 0o111 != 0;
                if writable && searchable {
                    continue;
                }
                blockers.push(DeleteBlocker {
                    path,
                    error: Some(if !writable {
                        "not writable".to_owned()
                    } else {
                        "directory not searchable".to_owned()
                    }),
                    mode: Some(mode),
                    uid: Some(metadata.uid()),
                    gid: Some(metadata.gid()),
                });
            }
            Err(source) => blockers.push(DeleteBlocker {
                path,
                error: Some(source.to_string()),
                mode: None,
                uid: None,
                gid: None,
            }),
        }
    }
    blockers
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use std::os::unix::fs::PermissionsExt as _;

    use super::*;
    use crate::action::feature::create::CreateInput;
    use crate::action::feature::create::create as create_action;
    use crate::action::feature::promote::{self, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    /// A hall with one seeded repo declared, a feature created, and the repo
    /// promoted (so a real worktree exists to tear down).
    fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

        create_action(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        (guard, root)
    }

    fn delete_input(name: &str) -> DeleteInput {
        DeleteInput {
            name: name.to_owned(),
        }
    }

    #[test]
    fn delete_removes_worktrees_the_feature_dir_and_plans() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        // A plan artifact to tear down alongside.
        fs::ensure_dir(&root.join("plans/checkout")).unwrap();
        fs::write_text(&root.join("plans/checkout/plan.md"), "# Plan\n").unwrap();

        let report = delete(&ctx, delete_input("checkout")).unwrap();

        assert!(report.is_clean());
        assert!(report.value.feature_removed);
        assert!(report.value.plans_removed);
        assert_eq!(report.value.worktrees.len(), 1);
        assert!(report.value.worktrees[0].removed);

        assert!(!fs::exists(&root.join(".ivar/features/checkout")).unwrap());
        assert!(!fs::exists(&root.join("plans/checkout")).unwrap());
        assert!(!fs::exists(&root.join(".ivar/repos/api/checkout")).unwrap());
    }

    #[test]
    fn delete_preflight_blocks_on_an_unwritable_path_and_mutates_nothing() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());

        // A directory with its write bits stripped — the preflight must name
        // it and refuse, leaving the feature fully intact.
        let planning = root.join(".ivar/features/checkout/planning");
        fs::ensure_dir(&planning).unwrap();
        fs::write_text(&planning.join("approvals.json"), "{}").unwrap();
        let original = fs_err::metadata(planning.as_std_path())
            .unwrap()
            .permissions()
            .mode();
        fs_err::set_permissions(
            planning.as_std_path(),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();

        let failure = delete(&ctx, delete_input("checkout")).unwrap_err();
        // Restore so TempDir can clean up.
        fs_err::set_permissions(
            planning.as_std_path(),
            std::fs::Permissions::from_mode(original),
        )
        .unwrap();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.delete_blocked");
        let blockers: Vec<DeleteBlocker> =
            serde_json::from_value(failure.details.clone().expect("blockers in details")).unwrap();
        assert!(
            blockers.iter().any(|blocker| blocker.path == planning),
            "blockers were: {blockers:?}"
        );
        assert_eq!(blockers[0].mode.unwrap() & 0o222, 0);
        // Nothing was mutated.
        assert!(fs::is_file(&root.join(".ivar/features/checkout/feature.json")).unwrap());
        assert!(fs::is_dir(&root.join(".ivar/repos/api/checkout")).unwrap());
    }

    #[test]
    fn delete_after_a_successful_delete_is_a_clean_refusal() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root.clone());
        delete(&ctx, delete_input("checkout")).unwrap();

        // The record is gone, so a second delete refuses cleanly — the system
        // is in a stable, fully-deleted state, and retrying is safe.
        let failure = delete(&ctx, delete_input("checkout")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_found");
    }

    #[test]
    fn delete_is_rejected_for_a_missing_feature() {
        let (_guard, root) = hall_with_promoted_feature();
        let ctx = Ctx::new(root);

        let failure = delete(&ctx, delete_input("ghost")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_found");
    }

    #[test]
    fn the_human_surface_names_what_was_deleted() {
        let outcome = DeleteOutcome {
            root: Utf8PathBuf::from("/hall"),
            name: FeatureName::new("checkout").unwrap(),
            worktrees: vec![WorktreeRemoval {
                repo: RepoName::new("api").unwrap(),
                removed: true,
                detail: None,
            }],
            feature_removed: true,
            plans_removed: true,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Deleted feature `checkout` in /hall\n"
        );
    }
}
