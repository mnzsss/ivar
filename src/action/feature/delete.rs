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
            Ok(()) => {
                // A branch holding a `/` — `feat/login` — nests the worktree
                // under a prefix directory that git does not know about and
                // will not take with it. Reclaim it, stopping at the repo dir
                // and at the first prefix another worktree still occupies.
                fs::prune_empty_parents(&worktree, &layout.repo_dir(repo));
                worktrees.push(WorktreeRemoval {
                    repo: repo.clone(),
                    removed: true,
                    detail: None,
                });
            }
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
#[path = "../../../tests/unit/action/feature/delete.rs"]
mod tests;
