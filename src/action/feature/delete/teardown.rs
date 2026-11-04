use camino::{Utf8Path, Utf8PathBuf};

use super::DeleteBlocker;
use crate::action::session::lookup as session_lookup;
use crate::domain::feature::WorktreeRemoval;
use crate::error::{Failure, Warning};
use crate::git::{self, Git};
use crate::infra::fs;

/// What deleting `feature` would destroy: its live sessions and every promoted
/// worktree with uncommitted or untracked changes, one sentence each. A
/// worktree or session list that cannot be read counts too, since it cannot
/// be proven safe.
pub(super) fn work_at_risk(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    feature: &crate::domain::feature::Feature,
) -> Vec<String> {
    let mut at_risk = match session_lookup::list_feature(layout, &feature.name) {
        Ok(sessions) => sessions
            .iter()
            .map(|session| format!("session `{}` is live", session.id))
            .collect(),
        Err(failure) => vec![format!("cannot check its sessions: {}", failure.what)],
    };
    for repo in feature.promotions.keys() {
        let bare = layout.repo_bare(repo);
        let dirty =
            git::lookup_worktree(git, &bare, feature.branch.as_str()).and_then(|worktree| {
                match worktree {
                    Some(worktree) if !worktree.prunable => git.worktree_dirty(&worktree.path),
                    _ => Ok(false),
                }
            });
        match dirty {
            Ok(false) => {}
            Ok(true) => at_risk.push(format!("`{repo}` has uncommitted or untracked changes")),
            Err(error) => at_risk.push(format!("cannot check `{repo}`: {error}")),
        }
    }
    at_risk
}

pub(super) fn teardown_worktrees(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    feature: &crate::domain::feature::Feature,
) -> Result<(Vec<WorktreeRemoval>, Vec<Warning>, bool), Failure> {
    let mut warnings = Vec::new();
    let mut worktrees = Vec::new();
    let mut all_worktrees_removed = true;
    for repo in feature.promotions.keys() {
        let bare = layout.repo_bare(repo);
        let worktree = git::lookup_worktree(git, &bare, feature.branch.as_str()).map_err(
            |error| match error {
                git::Error::Fs(_) => Failure::from(error),
                _ => Failure::failed("feature.delete_worktree_lookup_failed", error.to_string()),
            },
        )?;
        let Some(worktree) = worktree else {
            // Nothing materialised — nothing to remove.
            worktrees.push(WorktreeRemoval {
                repo: repo.clone(),
                removed: true,
                detail: None,
            });
            continue;
        };
        match git::remove_worktree_entry(git, &bare, &worktree) {
            Ok(()) => {
                // A branch holding a `/` — `feat/login` — nests the worktree
                // under a prefix directory that git does not know about and
                // will not take with it. Reclaim it, stopping at the repo dir
                // and at the first prefix another worktree still occupies.
                fs::prune_empty_parents(&worktree.path, &layout.repo_dir(repo));
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
    Ok((worktrees, warnings, all_worktrees_removed))
}

/// Walk `root` and report every path that cannot be removed.
///
/// A file is removable when its parent directory is writable and searchable, so only
/// directories are checked. The check reads mode bits directly rather than probing
/// `access(2)`, which lets it report why (mode, uid, gid) and keeps it honest as root,
/// where permission checks always answer yes.
pub(crate) fn collect_blockers(root: &Utf8Path) -> Vec<DeleteBlocker> {
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
                if !metadata.is_dir() {
                    continue;
                }
                let mode = metadata.mode();
                let writable = mode & 0o222 != 0;
                let searchable = mode & 0o111 != 0;
                if writable && searchable {
                    continue;
                }
                blockers.push(DeleteBlocker {
                    path,
                    error: Some(if !writable {
                        "not writable".to_owned()
                    } else {
                        "not searchable".to_owned()
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
