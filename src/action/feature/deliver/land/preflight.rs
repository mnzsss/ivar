//! Preflight validation and land plan construction for all promoted repositories.

use crate::domain::feature::{DeliveryPreview, Feature};
use crate::error::{Failure, FixAction};
use crate::git::Git;
use crate::store::layout::Layout;

use super::LandPlan;

/// Preflight check for landing: checks every promoted repo for blockers before any write.
/// Returns a list of [`LandPlan`]s on success, or the first [`Failure`] blocker.
pub fn preflight(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    preview: &DeliveryPreview,
) -> Result<Vec<LandPlan>, Failure> {
    if preview.repos.is_empty() {
        return Err(Failure::blocked(
            "deliver.land_no_repos",
            format!(
                "feature `{}` promotes no repositories to land",
                feature.name
            ),
        )
        .expected("at least one promoted repository to land")
        .actual(format!(
            "feature `{}` promotes 0 repositories",
            feature.name
        ))
        .fix(FixAction::safe(
            "deliver.promote_first",
            "Promote at least one repository before delivering.",
        )));
    }

    let mut plans = Vec::new();

    for repo in &preview.repos {
        let default_branch = repo.default_branch.as_ref().ok_or_else(|| {
            Failure::blocked(
                "deliver.declare_default_branch",
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
            feature_name: feature.name.to_string(),
        });
    }

    Ok(plans)
}
