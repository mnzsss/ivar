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

use crate::domain::name::FeatureName;
#[cfg(test)]
use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;

use super::super::discover_hall;
use super::relations;
use crate::action::Ctx;
use crate::action::session::lookup as session_lookup;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Descendants {
    Refuse,
    Ask,
    Consented,
}

/// What `ivar feature delete` needs.
#[derive(Debug, Clone)]
pub struct DeleteInput {
    /// The feature's name.
    pub name: String,
    /// Delete even with a live session or uncommitted or untracked changes
    /// in a promoted worktree, discarding them.
    pub force: bool,
    /// How to handle descendants when deleting a parent feature.
    pub descendants: Descendants,
}

pub use crate::domain::feature::WorktreeRemoval;

#[derive(Debug, Clone, Serialize)]
pub struct DeletedFeature {
    pub name: FeatureName,
    pub worktrees: Vec<WorktreeRemoval>,
    pub feature_removed: bool,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub descendants: Vec<DeletedFeature>,
}

impl WriteHuman for DeleteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.feature_removed {
            writeln!(w, "Deleted feature `{}` in {}", self.name, self.root)?;
            for desc in &self.descendants {
                if desc.feature_removed {
                    writeln!(w, "  Deleted subfeature `{}`", desc.name)?;
                }
            }
            Ok(())
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
            for desc in &self.descendants {
                if !desc.feature_removed {
                    writeln!(w, "  subfeature `{}`: record kept", desc.name)?;
                    for removal in &desc.worktrees {
                        if !removal.removed {
                            writeln!(w, "    {}: worktree not removed", removal.repo)?;
                        }
                    }
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
fn check_descendants_consent(
    ctx: &Ctx,
    name: &FeatureName,
    subtree: &[(usize, &crate::domain::feature::Feature)],
    descendants: Descendants,
    force: bool,
) -> Result<(), Failure> {
    if subtree.is_empty() {
        return Ok(());
    }
    match descendants {
        Descendants::Refuse => {
            let names = subtree
                .iter()
                .map(|(_, descendant)| descendant.name.to_string())
                .collect::<Vec<_>>();
            Err(Failure::blocked(
                "feature.has_descendants",
                format!(
                    "cannot delete feature `{name}`: it has {} descendant(s)",
                    subtree.len()
                ),
            )
            .expected("every descendant to be deleted first")
            .actual(format!("descendants: {}", names.join(", ")))
            .fix(FixAction::safe(
                "feature.delete_leaves_first",
                "Delete the descendants first, leaves first.",
            )))
        }
        Descendants::Ask => {
            let depths = std::iter::once(0)
                .chain(subtree.iter().map(|(d, _)| *d))
                .collect::<Vec<_>>();
            let prefixes = crate::action::feature::tree::tree_prefixes(&depths);
            let names = std::iter::once(name)
                .chain(subtree.iter().map(|(_, descendant)| &descendant.name));
            let rendered_lines = names
                .zip(prefixes)
                .map(|(feature_name, prefix)| format!("{prefix}{feature_name}"))
                .collect::<Vec<_>>();
            let rendered_tree = rendered_lines.join("\n");

            if !ctx.confirm.is_interactive() {
                let mut cmd = format!("ivar feature delete {name} --yes");
                if force {
                    cmd.push_str(" --force");
                }
                return Err(Failure::blocked(
                    "feature.delete_subtree_needs_consent",
                    format!(
                        "cannot delete feature `{name}`: it has {} descendant(s)",
                        subtree.len()
                    ),
                )
                .expected("explicit consent to delete the entire subtree")
                .actual(format!("subtree:\n{rendered_tree}"))
                .fix(
                    FixAction::unsafe_(
                        "feature.delete_yes",
                        "Pass `--yes` to consent to deleting the feature together with every subfeature.",
                    )
                    .command(cmd),
                ));
            }

            let question = format!("Delete `{name}` and its {} subfeature(s)?", subtree.len());
            let confirmed = ctx.confirm(&question, Some(&rendered_tree))?;
            if !confirmed {
                return Err(Failure::blocked(
                    "feature.delete_declined",
                    format!("delete declined for feature `{name}`"),
                ));
            }
            Ok(())
        }
        Descendants::Consented => Ok(()),
    }
}

fn preflight_subtree_clean(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    name: &FeatureName,
    all_nodes: &[&crate::domain::feature::Feature],
    subtree_is_empty: bool,
) -> Result<(), Failure> {
    let mut all_at_risk = Vec::new();
    for node in all_nodes {
        let at_risk = work_at_risk(layout, git, node);
        for msg in at_risk {
            all_at_risk.push(format!("`{}`: {msg}", node.name));
        }
    }
    if !all_at_risk.is_empty() {
        let mut cmd = format!("ivar feature delete {name} --force");
        if !subtree_is_empty {
            cmd = format!("ivar feature delete {name} --yes --force");
        }
        return Err(Failure::blocked(
            "feature.delete_unsaved_work",
            format!("cannot delete feature `{name}`: it has work that would be lost"),
        )
        .expected("no live session and a clean worktree in every promoted repo")
        .actual(all_at_risk.join("; "))
        .fix(
            FixAction::unsafe_(
                "feature.delete_force",
                "Commit or stop the work first, or discard it with `--force`.",
            )
            .command(cmd),
        ));
    }
    Ok(())
}

fn preflight_subtree_removable(
    layout: &crate::store::layout::Layout,
    name: &FeatureName,
    all_nodes: &[&crate::domain::feature::Feature],
) -> Result<(), Failure> {
    let mut all_blockers = Vec::new();
    for node in all_nodes {
        let blockers = collect_blockers(&layout.feature_dir(&node.name));
        all_blockers.extend(blockers);
    }
    if !all_blockers.is_empty() {
        let details = serde_json::to_value(&all_blockers).unwrap_or(serde_json::Value::Null);
        return Err(Failure::blocked(
            "feature.delete_blocked",
            format!(
                "cannot delete feature `{name}`: {} path(s) under its directory are not removable",
                all_blockers.len()
            ),
        )
        .expected("every directory under the feature directory to be writable and searchable")
        .actual(format!(
            "{} path(s) could not be removed — see details for paths, modes, and owners",
            all_blockers.len()
        ))
        .fix(FixAction::safe(
            "feature.fix_permissions",
            format!(
                "Fix the permissions named above, then run `ivar feature delete {name}` again."
            ),
        ))
        .details(details));
    }
    Ok(())
}

pub fn delete(ctx: &Ctx, input: DeleteInput) -> Outcome<DeleteOutcome> {
    let layout = discover_hall(ctx)?;
    let git = git::System;
    let name = FeatureName::new(input.name)?;

    // Keep read_feature for not-found error / fix action preservation
    let feature = relations::read_feature(&layout, &name)?;

    let all_features = relations::read_all(&layout)?;
    let map = relations::build_feature_map(&all_features);
    let subtree = relations::descendants_from_values(&map, &name);

    check_descendants_consent(ctx, &name, &subtree, input.descendants, input.force)?;

    // Preflight: whole subtree check before any mutation
    let all_nodes = subtree
        .iter()
        .map(|(_, f)| *f)
        .chain(std::iter::once(&feature))
        .collect::<Vec<_>>();

    if !input.force {
        preflight_subtree_clean(&layout, &git, &name, &all_nodes, subtree.is_empty())?;
    }

    preflight_subtree_removable(&layout, &name, &all_nodes)?;

    // Leaves-first teardown loop (reverse pre-order: descendants then root)
    let mut warnings = Vec::new();
    let mut deleted_descendants = Vec::new();
    let mut root_worktrees = Vec::new();
    let mut root_removed = false;

    let teardown_order = subtree
        .iter()
        .map(|(_, f)| *f)
        .rev()
        .chain(std::iter::once(&feature));

    for node in teardown_order {
        let is_root = node.name == name;
        let (worktrees, node_warnings, all_worktrees_removed) =
            teardown_worktrees(&layout, &git, node)?;
        warnings.extend(node_warnings);

        if !all_worktrees_removed {
            if is_root {
                root_worktrees = worktrees;
                root_removed = false;
            } else {
                deleted_descendants.push(DeletedFeature {
                    name: node.name.clone(),
                    worktrees,
                    feature_removed: false,
                });
            }
            // Stop at first failing node: ancestors stay untouched
            break;
        }

        fs::remove_path(&layout.feature_dir(&node.name)).map_err(|source| {
            Failure::failed(
                "feature.delete_dir_failed",
                format!("could not remove feature `{}`: {source}", node.name),
            )
        })?;

        let db_path = layout.ivar_dir().join("memory.db");
        if db_path.is_file()
            && let Ok(db) = crate::store::graph::db::GraphDb::open(db_path.as_std_path())
        {
            let _ = db.drop_feature_layers(node.name.as_str());
        }

        if is_root {
            root_worktrees = worktrees;
            root_removed = true;
        } else {
            deleted_descendants.push(DeletedFeature {
                name: node.name.clone(),
                worktrees,
                feature_removed: true,
            });
        }
    }

    Ok(Report::with_warnings(
        DeleteOutcome {
            root: layout.root().to_path_buf(),
            name,
            worktrees: root_worktrees,
            feature_removed: root_removed,
            descendants: deleted_descendants,
        },
        warnings,
    ))
}

pub fn fold_into_ancestors(ctx: &Ctx, targets: &[String]) -> Result<Vec<String>, Failure> {
    let layout = discover_hall(ctx)?;
    let all_features = relations::read_all(&layout)?;
    let map = relations::build_feature_map(&all_features);
    let target_set: std::collections::HashSet<&str> = targets.iter().map(String::as_str).collect();
    let mut folded = Vec::new();
    for target in targets {
        let is_descendant_of_target = if let Ok(fname) = FeatureName::new(target) {
            let mut current = map.get(&fname).and_then(|f| f.parent.as_ref());
            let mut found_ancestor = false;
            while let Some(parent_name) = current {
                if target_set.contains(parent_name.as_str()) {
                    found_ancestor = true;
                    break;
                }
                current = map.get(parent_name).and_then(|f| f.parent.as_ref());
            }
            found_ancestor
        } else {
            false
        };
        if !is_descendant_of_target {
            folded.push(target.clone());
        }
    }
    Ok(folded)
}

/// What deleting `feature` would destroy: its live sessions and every promoted
/// worktree with uncommitted or untracked changes, one sentence each. A
/// worktree or session list that cannot be read counts too, since it cannot
/// be proven safe.
fn work_at_risk(
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

fn teardown_worktrees(
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

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/delete.rs"]
mod tests;
