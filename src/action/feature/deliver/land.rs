//! Land mode execution: preflight validation, permission guard, fast-forward merge, and best-effort push.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::{DeliveryPreview, Feature};
use crate::domain::name::{BranchName, RepoName};
use crate::error::{Failure, FixAction, Warning};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::LandResult;

/// What Task 10 executes per repository.
#[derive(Debug, Clone)]
pub(crate) struct LandPlan {
    pub(crate) repo: RepoName,
    pub(crate) worktree: Utf8PathBuf,
    pub(crate) default_branch: BranchName,
    pub(crate) tip: String,
    pub(crate) remote: String,
    pub(crate) remote_default_tip: Option<String>,
    pub(crate) original_head: String,
}

/// Scope guard ensuring read-only default worktree permissions are always restored on drop.
pub(crate) struct WorktreeWriteGuard {
    lifted: Vec<(Utf8PathBuf, u32)>,
}

impl WorktreeWriteGuard {
    pub(crate) fn lift(worktrees: &[&Utf8Path]) -> Result<Self, Failure> {
        let mut guard = Self { lifted: Vec::new() };
        for &wt in worktrees {
            match crate::infra::fs::unix_mode(wt) {
                Ok(Some(mode)) if mode & 0o222 == 0 => {
                    if let Err(e) = crate::infra::fs::restore_write_bits(wt) {
                        return Err(Failure::failed(
                            "deliver.lift_write_bits_failed",
                            format!("could not lift write permissions on `{wt}`: {e}"),
                        )
                        .expected(format!("write permissions to be lifted on `{wt}`"))
                        .actual(format!("chmod failed: {e}"))
                        .fix(FixAction::safe(
                            "deliver.check_permissions",
                            format!("Ensure user has permission to modify permissions on `{wt}`."),
                        )));
                    }
                    guard.lifted.push((wt.to_path_buf(), mode));
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(Failure::failed(
                        "deliver.read_mode_failed",
                        format!("could not inspect permissions on `{wt}`: {e}"),
                    )
                    .expected(format!("path `{wt}` to exist and be readable"))
                    .actual(format!("fs error: {e}"))
                    .fix(FixAction::safe(
                        "deliver.check_path",
                        format!("Ensure `{wt}` exists and is accessible."),
                    )));
                }
            }
        }
        Ok(guard)
    }
}

impl Drop for WorktreeWriteGuard {
    fn drop(&mut self) {
        for (wt, orig_mode) in &self.lifted {
            let _ = crate::infra::fs::chmod(wt, *orig_mode);
        }
    }
}

/// Preflight check for landing: checks every promoted repo for blockers before any write.
/// Returns a list of [`LandPlan`]s on success, or the first [`Failure`] blocker.
pub(crate) fn preflight(
    git: &impl Git,
    layout: &Layout,
    _manifest: &Manifest,
    feature: &Feature,
    preview: &DeliveryPreview,
) -> Result<Vec<LandPlan>, Failure> {
    if preview.repos.is_empty() {
        return Err(Failure::blocked(
            "deliver.land_no_repos",
            format!("feature `{}` promotes no repositories to land", feature.name),
        )
        .expected("at least one promoted repository to land")
        .actual(format!("feature `{}` promotes 0 repositories", feature.name))
        .fix(FixAction::safe(
            "deliver.promote_first",
            "Promote at least one repository before delivering.",
        )));
    }

    let mut plans = Vec::new();

    for repo in &preview.repos {
        let default_branch = repo.default_branch.as_ref().ok_or_else(|| {
            Failure::blocked(
                "deliver.land_not_fast_forward",
                format!(
                    "default branch in repo `{}` is not declared in ivar.json",
                    repo.repo
                ),
            )
            .expected("a declared default branch in ivar.json")
            .actual("no default branch declared")
            .fix(FixAction::safe(
                "deliver.declare_default_branch",
                "Declare a default branch for the repository in ivar.json before landing.",
            ))
        })?;

        let default_worktree = layout.repo_worktree(&repo.repo, default_branch);

        if git.is_rebase_in_progress(&default_worktree)? {
            return Err(Failure::blocked(
                "deliver.land_rebase_in_progress",
                format!("a rebase is in progress in `{default_worktree}`"),
            )
            .expected("the default branch worktree to have no active rebase")
            .actual(format!("rebase in progress in `{default_worktree}`"))
            .fix(FixAction::safe(
                "deliver.finish_rebase_first",
                "Complete or abort the in-progress rebase before landing.",
            )));
        }

        if git.worktree_dirty(&default_worktree)? {
            return Err(Failure::blocked(
                "deliver.land_dirty_worktree",
                format!("the default worktree at `{default_worktree}` has uncommitted changes"),
            )
            .expected("the default worktree to be clean")
            .actual(format!("uncommitted changes in `{default_worktree}`"))
            .fix(FixAction::safe(
                "deliver.clean_worktree_first",
                "Commit or stash your work before landing.",
            )));
        }

        let bare = layout.repo_bare(&repo.repo);

        if !git.is_ancestor(&bare, default_branch.as_str(), feature.branch.as_str())? {
            return Err(Failure::blocked(
                "deliver.land_not_fast_forward",
                format!(
                    "default branch `{default_branch}` in repo `{}` cannot fast-forward to feature `{}`",
                    repo.repo, feature.name
                ),
            )
            .expected(format!(
                "default branch `{default_branch}` to fast-forward to `{}`",
                feature.name
            ))
            .actual(format!(
                "default branch `{default_branch}` has diverged or cannot fast-forward"
            ))
            .fix(
                FixAction::safe(
                    "deliver.rebase_first",
                    format!(
                        "Rebase the feature onto default first: `ivar feature rebase {}`.",
                        feature.name
                    ),
                )
                .command(format!("ivar feature rebase {}", feature.name)),
            ));
        }

        let tip = git.revision_commit(&bare, feature.branch.as_str())?;
        let original_head = git.head_commit(&default_worktree)?;

        plans.push(LandPlan {
            repo: repo.repo.clone(),
            worktree: default_worktree,
            default_branch: default_branch.clone(),
            tip,
            remote: repo.remote.clone(),
            remote_default_tip: repo.remote_default_tip.clone(),
            original_head,
        });
    }

    Ok(plans)
}

/// Executes the land plans: lifts write bits, performs fast-forward merge, best-effort pushes.
pub(crate) fn execute(
    git: &impl Git,
    layout: &Layout,
    plans: &[LandPlan],
    warnings: &mut Vec<Warning>,
) -> Result<Vec<LandResult>, Failure> {
    let worktrees: Vec<&Utf8Path> = plans.iter().map(|p| p.worktree.as_path()).collect();
    let _guard = WorktreeWriteGuard::lift(&worktrees)?;

    let mut results = Vec::new();

    for (i, plan) in plans.iter().enumerate() {
        let bare = layout.repo_bare(&plan.repo);

        // Re-check remote default branch tip against the remote tip recorded during preview.
        if matches!(
            (&plan.remote_default_tip, git.remote_branch_tip(&bare, &plan.remote, plan.default_branch.as_str())),
            (Some(expected), Ok(Some(ref current))) if current != expected
        ) {
            warnings.push(Warning::new(
                "deliver.land_remote_moved",
                plan.repo.as_str(),
                format!(
                    "the remote default branch `{}` has moved since preview; skipping repository",
                    plan.default_branch
                ),
            ));
            results.push(LandResult {
                repo: plan.repo.clone(),
                merged: false,
                pushed: false,
                detail: Some("remote default branch moved".to_owned()),
            });
            continue;
        }

        if let Err(e) = git.fast_forward_to(&plan.worktree, &plan.tip) {
            // D5 Rollback: reset all previously merged worktrees to original_head
            for (prev_plan, prev_res) in plans.iter().zip(results.iter()).take(i) {
                if prev_res.merged {
                    let _ = git.reset_hard(&prev_plan.worktree, &prev_plan.original_head);
                }
            }

            return Err(Failure::failed(
                "git.merge_ff_only_failed",
                format!(
                    "failed to fast-forward default branch `{}` in `{}`: {e}",
                    plan.default_branch, plan.repo
                ),
            )
            .expected(format!(
                "default branch `{}` to fast-forward to `{}`",
                plan.default_branch, plan.tip
            ))
            .actual(format!("git merge --ff-only failed: {e}"))
            .fix(FixAction::safe(
                "deliver.rebase_first",
                format!(
                    "Rebase the feature onto default first: `ivar feature rebase {}`.",
                    feature_name_from_plan(plan)
                ),
            )));
        }

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

fn feature_name_from_plan(plan: &LandPlan) -> &str {
    plan.worktree.file_name().unwrap_or("feature")
}
