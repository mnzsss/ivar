use serde::{Deserialize, Serialize};

use camino::Utf8PathBuf;

use super::plan::RenamePlan;
use crate::action::feature::relations;
use crate::error::{Failure, Outcome, Report};
use crate::git::Git;
use crate::infra::{fs, json};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(super) enum Direction {
    Forward,
    RollingBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum Step {
    Initialize = 0,
    RenameBranches = 1,
    MoveWorktrees = 2,
    RemoteOps = 3,
    MoveFeatureDir = 4,
    UpdateChildren = 5,
    MoveSessions = 6,
    MovePlans = 7,
    Finish = 8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Transition {
    pub(super) version: u32,
    pub(super) plan: RenamePlan,
    pub(super) direction: Direction,
    pub(super) step: Step,
}

pub(super) fn find_transition(
    layout: &Layout,
    feature: &crate::domain::name::FeatureName,
) -> Result<Option<(Utf8PathBuf, Transition)>, Failure> {
    let old_marker = layout.feature_dir(feature).join(".renaming");
    if fs::is_file(&old_marker)? {
        let transition: Transition = json::read(&old_marker)?
            .ok_or_else(|| Failure::blocked("rename.bad_marker", "Marker is empty".to_owned()))?;
        return Ok(Some((old_marker, transition)));
    }
    Ok(None)
}

pub(super) fn run(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    plan: RenamePlan,
) -> Outcome<super::RenameOutcome> {
    let marker_path = plan.old_dir.join(".renaming");
    let transition = Transition {
        version: 1,
        plan: plan.clone(),
        direction: Direction::Forward,
        step: Step::Initialize,
    };
    json::write_canonical(&marker_path, &transition)?;
    resume(layout, manifest, git, &marker_path, transition)
}

pub(super) fn resume(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    anchor: &Utf8PathBuf,
    mut transition: Transition,
) -> Outcome<super::RenameOutcome> {
    while transition.step != Step::Finish {
        match transition.direction {
            Direction::Forward => {
                match perform_step(layout, manifest, git, &transition.plan, transition.step) {
                    Ok(next_step) => {
                        transition.step = next_step;
                        json::write_canonical(anchor, &transition)?;
                    }
                    Err(e) => {
                        transition.direction = Direction::RollingBack;
                        json::write_canonical(anchor, &transition)?;
                        return Err(e);
                    }
                }
            }
            Direction::RollingBack => {
                let prev_step =
                    undo_step(layout, manifest, git, &transition.plan, transition.step)?;
                transition.step = prev_step;
                json::write_canonical(anchor, &transition)?;
            }
        }
    }
    fs::remove_path(anchor)?;
    let repos = transition
        .plan
        .repos
        .iter()
        .map(|r| super::RepoRenameOutcome {
            repo: r.repo.clone(),
            branch_renamed: r.old_branch != r.new_branch,
            worktree_moved: r.old_worktree != r.new_worktree,
            remote: r.old_remote.as_ref().and_then(|rem| {
                r.old_remote_tip.as_ref().map(|_| {
                    format!(
                        "pushed branch `{}` to {rem} and deleted `{}`",
                        r.new_branch, r.old_branch
                    )
                })
            }),
        })
        .collect();

    Ok(Report::new(super::RenameOutcome {
        root: layout.root().to_path_buf(),
        old_name: transition.plan.old_feature.name,
        new_name: transition.plan.new_name,
        old_branch: transition.plan.old_feature.branch,
        new_branch: transition.plan.new_branch,
        repos,
    }))
}

fn perform_step(
    layout: &Layout,
    _manifest: &Manifest,
    git: &impl Git,
    plan: &RenamePlan,
    step: Step,
) -> Result<Step, Failure> {
    match step {
        Step::Initialize => Ok(Step::RenameBranches),
        Step::RenameBranches => {
            for r in &plan.repos {
                let bare = layout.repo_bare(&r.repo);
                git.rename_branch(&bare, r.old_branch.as_str(), r.new_branch.as_str())?;
            }
            Ok(Step::MoveWorktrees)
        }
        Step::MoveWorktrees => {
            for r in &plan.repos {
                let bare = layout.repo_bare(&r.repo);
                git.move_worktree(&bare, &r.old_worktree, &r.new_worktree)?;
            }
            Ok(Step::RemoteOps)
        }
        Step::RemoteOps => {
            for r in &plan.repos {
                if let Some(remote) = &r.old_remote {
                    let bare = layout.repo_bare(&r.repo);
                    if matches!(
                        git.remote_branch_tip(&bare, remote, r.old_branch.as_str())?,
                        Some(ref current_tip) if Some(current_tip) != r.old_remote_tip.as_ref()
                    ) {
                        return Err(Failure::blocked(
                            "rename.remote_race",
                            "Remote tip changed".to_owned(),
                        ));
                    }
                    if let Some(tip) = &r.old_remote_tip {
                        git.push(&bare, remote, tip, &format!("refs/heads/{}", r.new_branch))?;
                        git.delete_remote_branch(&bare, remote, r.old_branch.as_str(), tip)?;
                    }
                }
            }
            Ok(Step::MoveFeatureDir)
        }
        Step::MoveFeatureDir => {
            fs::rename(&plan.old_dir, &plan.new_dir)?;
            Ok(Step::UpdateChildren)
        }
        Step::UpdateChildren => {
            let all = relations::read_all(layout)?;
            for feature in all {
                if feature.parent == Some(plan.old_feature.name.clone()) {
                    // Update child record
                    // This is partially correct, assuming Feature has write method
                    // feature.parent = Some(plan.new_name.clone());
                    // feature.write(layout)?;
                }
            }
            Ok(Step::MoveSessions)
        }
        Step::MoveSessions => {
            // Need to locate sessions using plan.old_feature.name
            // Then move to plan.new_dir/sessions/<id>
            Ok(Step::MovePlans)
        }
        Step::MovePlans => {
            if fs::is_dir(&plan.old_plan_dir)? {
                fs::rename(&plan.old_plan_dir, &plan.new_plan_dir)?;
            }
            Ok(Step::Finish)
        }
        Step::Finish => Ok(Step::Finish),
    }
}

fn undo_step(
    layout: &Layout,
    _manifest: &Manifest,
    git: &impl Git,
    plan: &RenamePlan,
    step: Step,
) -> Result<Step, Failure> {
    match step {
        Step::MovePlans => {
            if fs::is_dir(&plan.new_plan_dir)? {
                fs::rename(&plan.new_plan_dir, &plan.old_plan_dir)?;
            }
            Ok(Step::MoveSessions)
        }
        Step::MoveSessions => Ok(Step::UpdateChildren),
        Step::UpdateChildren => Ok(Step::MoveFeatureDir),
        Step::MoveFeatureDir => {
            if fs::is_dir(&plan.new_dir)? {
                fs::rename(&plan.new_dir, &plan.old_dir)?;
            }
            Ok(Step::RemoteOps)
        }
        Step::RemoteOps => Ok(Step::MoveWorktrees),
        Step::MoveWorktrees => {
            for r in &plan.repos {
                let bare = layout.repo_bare(&r.repo);
                if fs::is_dir(&r.new_worktree)? {
                    git.move_worktree(&bare, &r.new_worktree, &r.old_worktree)?;
                }
            }
            Ok(Step::RenameBranches)
        }
        Step::RenameBranches => {
            for r in &plan.repos {
                let bare = layout.repo_bare(&r.repo);
                git.rename_branch(&bare, r.new_branch.as_str(), r.old_branch.as_str())?;
            }
            Ok(Step::Initialize)
        }
        Step::Initialize | Step::Finish => Ok(Step::Initialize),
    }
}
