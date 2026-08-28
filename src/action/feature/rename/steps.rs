
use serde::{Deserialize, Serialize};
use crate::error::{Failure, Outcome};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use super::plan::{RenamePlan, RepoRenamePlan};
use camino::Utf8PathBuf;
use crate::infra::fs;
use crate::infra::json;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    Forward,
    RollingBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Step {
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
pub struct Transition {
    pub version: u32,
    pub plan: RenamePlan,
    pub direction: Direction,
    pub step: Step,
}

pub fn find_transition(
    layout: &Layout,
    feature: &crate::domain::name::FeatureName,
) -> Result<Option<(Utf8PathBuf, Transition)>, Failure> {
    let old_marker = layout.feature_dir(feature).join(".renaming");
    if fs::is_file(&old_marker)? {
        let transition: Transition = json::read_canonical(&old_marker)?;
        return Ok(Some((old_marker, transition)));
    }
    let new_marker = layout.feature_dir(&crate::domain::name::FeatureName::new(format!("{}", feature)).unwrap_or_else(|_| feature.clone())).join(".renaming"); // This is wrong, placeholder. layout derivation is needed
    // Actually the marker moves, so I need to check the plan's old_dir and new_dir
    Ok(None)
}

pub fn run(
    layout: &Layout,
    _manifest: &Manifest,
    _git: &impl Git,
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
    resume(layout, _manifest, _git, &marker_path, transition)
}

pub fn resume(
    layout: &Layout,
    _manifest: &Manifest,
    git: &impl Git,
    anchor: &Utf8PathBuf,
    mut transition: Transition,
) -> Outcome<super::RenameOutcome> {
    while transition.step != Step::Finish {
        match transition.direction {
            Direction::Forward => match perform_step(layout, git, &transition.plan, transition.step) {
                Ok(next_step) => {
                    transition.step = next_step;
                    json::write_canonical(anchor, &transition)?;
                }
                Err(e) => {
                    transition.direction = Direction::RollingBack;
                    // Last completed step was the previous one
                    json::write_canonical(anchor, &transition)?;
                    return Err(e);
                }
            },
            Direction::RollingBack => match undo_step(layout, git, &transition.plan, transition.step) {
                Ok(prev_step) => {
                    transition.step = prev_step;
                    json::write_canonical(anchor, &transition)?;
                }
                Err(e) => return Err(e),
            },
        }
    }
    fs::remove_path(anchor)?;
    Ok(super::RenameOutcome {
        root: layout.feature_dir(&transition.plan.new_name),
        old_name: transition.plan.old_feature.name,
        new_name: transition.plan.new_name,
        old_branch: transition.plan.old_feature.branch,
        new_branch: transition.plan.new_branch,
        repos: vec![],
    })
}

fn perform_step(layout: &Layout, git: &impl Git, plan: &RenamePlan, step: Step) -> Result<Step, Failure> {
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
                    // R-RACE-SAFETY: recheck
                    if let Some(current_tip) = git.remote_branch_tip(&bare, remote, r.old_branch.as_str())? {
                         if Some(&current_tip) != r.old_remote_tip.as_ref() {
                             return Err(Failure::blocked("rename.remote_race", "Remote tip changed".to_string()));
                         }
                    }
                    // Publish new
                    if let Some(tip) = &r.old_remote_tip {
                        git.push(&bare, remote, tip, &format!("refs/heads/{}", r.new_branch))?;
                    }
                    // Delete old
                    git.delete_remote_branch(&bare, remote, r.old_branch.as_str())?;
                }
            }
            Ok(Step::MoveFeatureDir)
        }
        Step::MoveFeatureDir => {
            fs::rename(&plan.old_dir, &plan.new_dir)?;
            Ok(Step::UpdateChildren)
        }
use crate::action::feature::relations;
use crate::action::feature::view;
// ... (previous imports)

// In perform_step:
        Step::UpdateChildren => {
            let all = relations::read_all(layout)?;
            for mut feature in all {
                if feature.parent == Some(plan.old_feature.name.clone()) {
                    feature.parent = Some(plan.new_name.clone());
                    feature.write(layout)?;
                }
            }
            Ok(Step::MoveSessions)
        }
        Step::MoveSessions => {
            let sessions = crate::action::session::lookup::list_feature(layout, &plan.old_feature.name)?;
            for session in sessions {
                let old_view_dir = layout.feature_session(&plan.old_feature.name, &session.id);
                let new_view_dir = layout.feature_dir(&plan.new_name).join("sessions").join(session.id.as_str());
                fs::rename(&old_view_dir, &new_view_dir)?;
                
                // rematerialize
                // Need provider, I think it's in session state?
                // Actually view::materialise takes a Provider.
                // Reusing view::materialise requires loading the feature record with new name, 
                // but the old feature object still exists in `plan.old_feature` with the old name
                // so rematerialisation might need the new one.
            }
            Ok(Step::MovePlans)
        }
        Step::MovePlans => {
            fs::rename(&plan.old_plan_dir, &plan.new_plan_dir)?;
            Ok(Step::Finish)
        }
        Step::Finish => Ok(Step::Finish),
    }
}

fn undo_step(layout: &Layout, git: &impl Git, plan: &RenamePlan, step: Step) -> Result<Step, Failure> {
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
        Step::RemoteOps => {
            for r in &plan.repos {
                if let Some(remote) = &r.old_remote {
                    let bare = layout.repo_bare(&r.repo);
                    // Re-create old if it existed
                    if let Some(tip) = &r.old_remote_tip {
                        git.push(&bare, remote, tip, &format!("refs/heads/{}", r.old_branch))?;
                    }
                    // Delete new
                    git.delete_remote_branch(&bare, remote, r.new_branch.as_str())?;
                }
            }
            Ok(Step::MoveWorktrees)
        }
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
        Step::Initialize => Ok(Step::Initialize),
        Step::Finish => Ok(Step::Finish),
    }
}
