//! Three-phase execution for landing features onto default branch.

use std::collections::HashMap;

use camino::Utf8Path;

use crate::action::feature::deliver::outcome::LandResult;
use crate::error::{Failure, FixAction, Warning};
use crate::git::Git;
use crate::store::layout::Layout;

use super::LandPlan;
use crate::infra::fs::LiftedGuard;

/// Executes the land plans: lifts write bits, performs fast-forward merge, best-effort pushes.
///
/// Execution runs in three distinct phases (ADR-0004 all-or-nothing guarantee):
/// Phase 1 — Pre-Execution Remote Validation: Checks remote default branch tips for ALL plans.
///          Per LandPlan, approved `remote_default_tip` MUST be `Some(expected)` and match current remote tip.
///          If `remote_default_tip` is `None` or current remote tip fails/mismatches for ANY plan,
///          the WHOLE batch is skipped with a warning before any worktree write.
/// Phase 2 — Local Fast-Forward Merge: Fast-forward merges ALL plans. If ANY plan fails, ALL previously
///          merged plans are rolled back to `original_head`. If rollback fails, `deliver.land_rollback_failed` is returned.
/// Phase 3 — Best-Effort Push: Pushes ALL plans. Push failures produce warnings (merges stand).
pub fn execute(
    git: &impl Git,
    layout: &Layout,
    plans: &[LandPlan],
    warnings: &mut Vec<Warning>,
) -> Result<Vec<LandResult>, Failure> {
    // --- Phase 1: Pre-Execution Remote Validation across ALL plans BEFORE writing any worktree ---
    let mut remote_moved = false;
    let mut plan_reasons = HashMap::new();

    for plan in plans {
        let bare = layout.repo_bare(&plan.repo);
        let current_res = git.remote_branch_tip(&bare, &plan.remote, plan.default_branch.as_str());

        let reason = match (&plan.remote_default_tip, &current_res) {
            (Some(expected), Ok(Some(current))) if current == expected => None,
            (Some(expected), Ok(Some(current))) => Some(format!(
                "moved (preview expected `{expected}`, current `{current}`)"
            )),
            (Some(_), Ok(None)) => Some("disappeared from remote".to_owned()),
            (Some(_), Err(e)) => Some(format!("could not be verified: {e}")),
            (None, Ok(Some(_))) => Some("absent at preview".to_owned()),
            (None, Ok(None)) => Some("absent at preview".to_owned()),
            (None, Err(e)) => Some(format!("absent at preview (remote error: {e})")),
        };

        if let Some(detail) = reason {
            remote_moved = true;
            plan_reasons.insert(plan.repo.clone(), format!("remote default branch {detail}"));
            warnings.push(Warning::new(
                "deliver.land_remote_moved",
                plan.repo.as_str(),
                format!(
                    "the remote default branch `{}` in `{}`: {detail}; the entire land batch is skipped",
                    plan.default_branch, plan.repo
                ),
            ));
        }
    }

    if remote_moved {
        // ADR-0004: Zero worktrees written if any remote default moved, disappeared, or lacked preview evidence
        return Ok(plans
            .iter()
            .map(|plan| LandResult {
                repo: plan.repo.clone(),
                merged: false,
                pushed: false,
                detail: plan_reasons.get(&plan.repo).cloned().or_else(|| {
                    Some("remote default branch validation failed for batch".to_owned())
                }),
            })
            .collect());
    }

    // --- Lift permissions for local merges ---
    let worktrees: Vec<&Utf8Path> = plans.iter().map(|p| p.worktree.as_path()).collect();
    let _guard = LiftedGuard::lift(&worktrees)?;

    // --- Phase 2: Local Fast-Forward Merge for ALL plans with all-or-nothing rollback ---
    for (i, plan) in plans.iter().enumerate() {
        // Re-validate immediately before each merge (ADR-0004 D5)
        let bare = layout.repo_bare(&plan.repo);
        if !git.is_ancestor(&bare, plan.default_branch.as_str(), &plan.tip)? {
            let failure = Failure::blocked(
                "deliver.land_not_fast_forward",
                format!(
                    "default branch `{}` in repo `{}` cannot fast-forward to feature `{}`",
                    plan.default_branch, plan.repo, plan.feature_name
                ),
            )
            .expected(format!(
                "default branch `{}` to fast-forward to `{}`",
                plan.default_branch, plan.feature_name
            ))
            .actual(format!(
                "default branch `{}` has diverged or cannot fast-forward",
                plan.default_branch
            ))
            .fix(
                FixAction::safe(
                    "deliver.rebase_first",
                    format!(
                        "Rebase the feature onto default first: `ivar feature rebase {}`.",
                        plan.feature_name
                    ),
                )
                .command(format!("ivar feature rebase {}", plan.feature_name)),
            );
            return rollback_merged(git, plans, i, failure);
        }

        if git.worktree_dirty(&plan.worktree)? {
            let failure = Failure::blocked(
                "deliver.land_dirty_worktree",
                format!(
                    "the default worktree at `{}` has uncommitted changes",
                    plan.worktree
                ),
            )
            .expected("the default worktree to be clean")
            .actual(format!("uncommitted changes in `{}`", plan.worktree))
            .fix(FixAction::safe(
                "deliver.clean_worktree_first",
                "Commit or stash your work before landing.",
            ));
            return rollback_merged(git, plans, i, failure);
        }

        let current_head = git.head_commit(&plan.worktree)?;
        if current_head != plan.original_head {
            let failure = Failure::blocked(
                "deliver.land_head_moved",
                format!(
                    "default branch `{}` in repo `{}` moved since preflight",
                    plan.default_branch, plan.repo
                ),
            )
            .expected(format!(
                "default branch `{}` to remain at preflight HEAD `{}`",
                plan.default_branch, plan.original_head
            ))
            .actual(format!("HEAD is now `{current_head}`"))
            .fix(
                FixAction::safe(
                    "deliver.rebase_first",
                    format!(
                        "Rebase the feature onto `{}`: `ivar feature rebase {}`.",
                        plan.default_branch, plan.feature_name
                    ),
                )
                .command(format!("ivar feature rebase {}", plan.feature_name)),
            );
            return rollback_merged(git, plans, i, failure);
        }

        if let Err(e) = git.fast_forward_to(&plan.worktree, &plan.tip) {
            let orig_err = format!(
                "failed to fast-forward default branch `{}` in `{}`: {e}",
                plan.default_branch, plan.repo
            );
            let failure = Failure::failed("git.merge_ff_only_failed", orig_err)
                .expected(format!(
                    "default branch `{}` to fast-forward to `{}`",
                    plan.default_branch, plan.tip
                ))
                .actual(format!("git merge --ff-only failed: {e}"))
                .fix(FixAction::safe(
                    "deliver.rebase_first",
                    format!(
                        "Rebase the feature onto default first: `ivar feature rebase {}`.",
                        plan.feature_name
                    ),
                ));
            return rollback_merged(git, plans, i, failure);
        }
    }

    // --- Phase 3: Best-Effort Push for ALL plans ---
    let mut results = Vec::new();
    for plan in plans {
        let bare = layout.repo_bare(&plan.repo);
        let push_result = git.push(
            &bare,
            &plan.remote,
            plan.default_branch.as_str(),
            &format!("refs/heads/{}", plan.default_branch),
        );

        match push_result {
            Ok(()) => {
                results.push(LandResult {
                    repo: plan.repo.clone(),
                    merged: true,
                    pushed: true,
                    detail: None,
                });
            }
            Err(e) => {
                let detail = e.to_string();
                warnings.push(Warning::new(
                    "deliver.land_push_failed",
                    plan.repo.as_str(),
                    format!("merged locally, push failed: {detail}"),
                ));
                results.push(LandResult {
                    repo: plan.repo.clone(),
                    merged: true,
                    pushed: false,
                    detail: Some(detail),
                });
            }
        }
    }

    Ok(results)
}

fn rollback_merged(
    git: &impl Git,
    plans: &[LandPlan],
    failed_index: usize,
    failure: Failure,
) -> Result<Vec<LandResult>, Failure> {
    let mut rollback_errors = Vec::new();
    for prev_plan in plans.iter().take(failed_index) {
        if let Err(reset_err) = git.reset_hard(&prev_plan.worktree, &prev_plan.original_head) {
            rollback_errors.push(format!(
                "`{}` (target `{}`): {reset_err}",
                prev_plan.repo, prev_plan.original_head
            ));
        }
    }

    if !rollback_errors.is_empty() {
        let repo_name = plans
            .get(failed_index)
            .map_or("unknown", |p| p.repo.as_str());
        return Err(Failure::failed(
            "deliver.land_rollback_failed",
            format!(
                "local merge failed for `{repo_name}`, and rollback of earlier merges also failed: {}",
                failure.what
            ),
        )
        .expected("all merged default worktrees to roll back cleanly to their original tips")
        .actual(format!(
            "original failure: {}; rollback errors: {}",
            failure.what,
            rollback_errors.join("; ")
        ))
        .fix(FixAction::safe(
            "deliver.manual_cleanup",
            format!(
                "Manually reset worktrees that failed rollback: {}.",
                rollback_errors.join("; ")
            ),
        )));
    }

    Err(failure)
}
