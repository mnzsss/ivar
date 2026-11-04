use super::*;

pub(super) fn teardown_descendants(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    descendants: &[(Feature, WorktreeLookups)],
) -> Result<(Vec<CleanupDescendantOutcome>, Vec<Warning>, bool), Failure> {
    let mut warnings = Vec::new();
    let mut descendant_outcomes = Vec::new();
    for (node, node_worktrees) in descendants.iter().rev() {
        let (worktrees, node_warnings, worktrees_removed) =
            teardown_worktrees(layout, git, node, node_worktrees)?;
        let (branches, branch_warnings, branches_deleted) =
            teardown_branches(layout, git, node, &worktrees);
        warnings.extend(node_warnings);
        warnings.extend(branch_warnings);
        let mut removed = false;
        if worktrees_removed && branches_deleted {
            match remove_node_dir(layout, &node.name) {
                Ok(warning) => {
                    if let Some(w) = warning {
                        warnings.push(w);
                    }
                    removed = true;
                }
                Err(failure) => {
                    warnings.push(Warning::new(
                        "feature.cleanup_dir_failed",
                        node.name.as_str(),
                        failure.what.clone(),
                    ));
                }
            }
        }
        descendant_outcomes.push(CleanupDescendantOutcome {
            feature: node.name.clone(),
            branch: node.branch.clone(),
            worktrees,
            branches,
            feature_removed: removed,
        });
        if !removed {
            return Ok((descendant_outcomes, warnings, false));
        }
    }
    Ok((descendant_outcomes, warnings, true))
}

pub(super) fn remove_node_dir(
    layout: &crate::store::layout::Layout,
    name: &FeatureName,
) -> Result<Option<Warning>, Failure> {
    fs::remove_path(&layout.feature_dir(name)).map_err(|source| {
        Failure::failed(
            "feature.cleanup_dir_failed",
            format!("could not remove feature `{name}`: {source}"),
        )
    })?;
    Ok(super::super::drop_layers_or_warn(layout, name))
}

pub(super) fn preflight_deletable(
    layout: &crate::store::layout::Layout,
    feature: &Feature,
    descendants: &[(Feature, WorktreeLookups)],
) -> Result<(), Failure> {
    let mut all_blockers = Vec::new();
    all_blockers.extend(delete::collect_blockers(&layout.feature_dir(&feature.name)));
    for (node, _) in descendants {
        all_blockers.extend(delete::collect_blockers(&layout.feature_dir(&node.name)));
    }
    if !all_blockers.is_empty() {
        let details = serde_json::to_value(&all_blockers).unwrap_or(serde_json::Value::Null);
        return Err(Failure::blocked(
            "feature.delete_blocked",
            format!(
                "cannot clean up feature `{}`: {} path(s) under its directory are not removable",
                feature.name,
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
            "Fix the permissions named above, then run apply again.",
        ))
        .details(details));
    }

    Ok(())
}

pub(super) fn teardown_worktrees(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    feature: &Feature,
    worktrees: &WorktreeLookups,
) -> Result<(Vec<WorktreeRemoval>, Vec<Warning>, bool), Failure> {
    let mut warnings = Vec::new();
    let mut worktree_removals = Vec::new();
    let mut all_worktrees_removed = true;

    for repo in feature.promotions.keys() {
        let worktree = match worktrees.get(repo) {
            Some(Ok(entry)) => entry.as_ref(),
            Some(Err(failure)) => return Err(failure.clone()),
            None => None,
        };
        let Some(worktree) = worktree else {
            worktree_removals.push(WorktreeRemoval {
                repo: repo.clone(),
                removed: true,
                detail: None,
            });
            continue;
        };
        match git::remove_worktree_entry(git, &layout.repo_bare(repo), worktree) {
            Ok(()) => {
                fs::prune_empty_parents(&worktree.path, &layout.repo_dir(repo));
                worktree_removals.push(WorktreeRemoval {
                    repo: repo.clone(),
                    removed: true,
                    detail: None,
                });
            }
            Err(error) => {
                all_worktrees_removed = false;
                let detail = error.to_string();
                warnings.push(Warning::new(
                    "feature.cleanup_worktree_failed",
                    repo.as_str(),
                    detail.clone(),
                ));
                worktree_removals.push(WorktreeRemoval {
                    repo: repo.clone(),
                    removed: false,
                    detail: Some(detail),
                });
            }
        }
    }

    Ok((worktree_removals, warnings, all_worktrees_removed))
}

pub(super) fn teardown_branches(
    layout: &crate::store::layout::Layout,
    git: &impl Git,
    feature: &Feature,
    worktree_removals: &[WorktreeRemoval],
) -> (Vec<BranchDeletion>, Vec<Warning>, bool) {
    let mut warnings = Vec::new();
    let mut branch_deletions = Vec::new();
    let mut all_branches_deleted = true;

    for (repo, removal) in feature.promotions.keys().zip(worktree_removals) {
        if !removal.removed {
            all_branches_deleted = false;
            branch_deletions.push(BranchDeletion {
                repo: repo.clone(),
                deleted: false,
                detail: Some("worktree removal failed".to_owned()),
            });
            continue;
        }

        let bare = layout.repo_bare(repo);
        let branch_exists = git.revision_commit(&bare, feature.branch.as_str()).is_ok();
        if !branch_exists {
            // Already absent — idempotent success
            branch_deletions.push(BranchDeletion {
                repo: repo.clone(),
                deleted: true,
                detail: None,
            });
            continue;
        }

        match git.delete_branch(&bare, feature.branch.as_str()) {
            Ok(()) => {
                branch_deletions.push(BranchDeletion {
                    repo: repo.clone(),
                    deleted: true,
                    detail: None,
                });
            }
            Err(error) => {
                all_branches_deleted = false;
                let detail = error.to_string();
                warnings.push(Warning::new(
                    "feature.cleanup_branch_failed",
                    repo.as_str(),
                    detail.clone(),
                ));
                branch_deletions.push(BranchDeletion {
                    repo: repo.clone(),
                    deleted: false,
                    detail: Some(detail),
                });
            }
        }
    }

    (branch_deletions, warnings, all_branches_deleted)
}
