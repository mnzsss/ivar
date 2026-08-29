use serde::{Deserialize, Serialize};

use camino::Utf8PathBuf;

use super::plan::RenamePlan;
use crate::action::session::{lookup, view};
use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
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

pub(super) fn marker_path(plan: &RenamePlan) -> Utf8PathBuf {
    if plan.old_feature.name != plan.new_name && fs::is_dir(&plan.new_dir).unwrap_or(false) {
        plan.new_dir.join(".renaming")
    } else {
        plan.old_dir.join(".renaming")
    }
}

pub(super) fn find_transition(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Option<(Utf8PathBuf, Transition)>, Failure> {
    let features_dir = layout.features_dir();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let marker = entry.join(".renaming");
            if fs::is_file(&marker)?
                && let Ok(Some(transition)) = json::read::<Transition>(&marker)
                && (transition.plan.old_feature.name == *feature
                    || transition.plan.new_name == *feature)
            {
                return Ok(Some((marker, transition)));
            }
        }
    }
    Ok(None)
}

pub(super) fn run(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    plan: RenamePlan,
) -> Outcome<super::RenameOutcome> {
    let initial_marker = plan.old_dir.join(".renaming");
    let transition = Transition {
        version: 1,
        plan: plan.clone(),
        direction: Direction::Forward,
        step: Step::Initialize,
    };
    json::write_canonical(&initial_marker, &transition)?;
    resume(layout, manifest, git, &initial_marker, transition)
}

pub(super) fn resume(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    anchor: &Utf8PathBuf,
    mut transition: Transition,
) -> Outcome<super::RenameOutcome> {
    let mut current_anchor = anchor.clone();
    while transition.step != Step::Finish {
        match transition.direction {
            Direction::Forward => {
                match perform_step(layout, manifest, git, &transition.plan, transition.step) {
                    Ok(next_step) => {
                        transition.step = next_step;
                        let next_anchor = marker_path(&transition.plan);
                        json::write_canonical(&next_anchor, &transition)?;
                        current_anchor = next_anchor;
                    }
                    Err(e) => {
                        transition.direction = Direction::RollingBack;
                        json::write_canonical(&current_anchor, &transition)?;
                        return Err(e);
                    }
                }
            }
            Direction::RollingBack => {
                if transition.step == Step::Initialize {
                    if fs::is_file(&current_anchor)? {
                        fs::remove_path(&current_anchor)?;
                    }
                    return Err(Failure::blocked(
                        "rename.rolled_back",
                        "Rename operation failed and was rolled back".to_owned(),
                    ));
                }
                let prev_step =
                    undo_step(layout, manifest, git, &transition.plan, transition.step)?;
                transition.step = prev_step;
                let next_anchor = marker_path(&transition.plan);
                json::write_canonical(&next_anchor, &transition)?;
                current_anchor = next_anchor;
            }
        }
    }

    perform_step(layout, manifest, git, &transition.plan, Step::Finish)?;

    let final_anchor = marker_path(&transition.plan);
    if fs::is_file(&final_anchor)? {
        fs::remove_path(&final_anchor)?;
    }

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

pub(super) fn perform_step(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    plan: &RenamePlan,
    step: Step,
) -> Result<Step, Failure> {
    match step {
        Step::Initialize => Ok(Step::RenameBranches),
        Step::RenameBranches => {
            for r in &plan.repos {
                if r.old_branch != r.new_branch {
                    let bare = layout.repo_bare(&r.repo);
                    if git.revision_commit(&bare, r.old_branch.as_str()).is_ok() {
                        git.rename_branch(&bare, r.old_branch.as_str(), r.new_branch.as_str())?;
                    }
                }
            }
            Ok(Step::MoveWorktrees)
        }
        Step::MoveWorktrees => {
            for r in &plan.repos {
                if r.old_worktree != r.new_worktree {
                    let bare = layout.repo_bare(&r.repo);
                    if fs::is_dir(&r.old_worktree)? {
                        git.move_worktree(&bare, &r.old_worktree, &r.new_worktree)?;
                    }
                }
            }
            Ok(Step::RemoteOps)
        }
        Step::RemoteOps => {
            for r in &plan.repos {
                if r.old_branch != r.new_branch
                    && let Some(remote) = &r.old_remote
                {
                    let bare = layout.repo_bare(&r.repo);
                    let old_tip = git.remote_branch_tip(&bare, remote, r.old_branch.as_str())?;
                    let new_tip = git.remote_branch_tip(&bare, remote, r.new_branch.as_str())?;
                    if old_tip.is_none() && new_tip.is_some() {
                        continue;
                    }
                    if new_tip.is_some() {
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
            if plan.old_feature.name != plan.new_name {
                if fs::is_dir(&plan.old_dir)? {
                    fs::rename(&plan.old_dir, &plan.new_dir)?;
                }
                if matches!(fs::read_symlink(&plan.old_dir)?, fs::SymlinkTarget::Absent) {
                    fs::create_symlink(&plan.new_dir, &plan.old_dir)?;
                }
            }
            let mut feature = plan.old_feature.clone();
            feature.name = plan.new_name.clone();
            feature.branch = plan.new_branch.clone();
            feature.write(layout)?;
            Ok(Step::UpdateChildren)
        }
        Step::UpdateChildren => {
            if plan.old_feature.name != plan.new_name {
                let features_dir = layout.features_dir();
                if fs::is_dir(&features_dir)? {
                    for entry in fs::read_dir(&features_dir)? {
                        if matches!(fs::read_symlink(&entry)?, fs::SymlinkTarget::Target(_)) {
                            continue;
                        }
                        let Some(name) = entry.file_name() else {
                            continue;
                        };
                        let Ok(feature_name) = FeatureName::new(name.to_owned()) else {
                            continue;
                        };
                        if let Some(mut feature) = Feature::read(layout, &feature_name)?
                            && feature.name == feature_name
                            && feature.parent == Some(plan.old_feature.name.clone())
                        {
                            feature.parent = Some(plan.new_name.clone());
                            feature.write(layout)?;
                        }
                    }
                }
            }
            Ok(Step::MoveSessions)
        }
        Step::MoveSessions => {
            let feature_novo = Feature::read(layout, &plan.new_name)?;
            let sessions = lookup::list_feature(layout, &plan.new_name)?;
            for session_ref in sessions {
                if let Some(mut s) = session_ref.state {
                    s.feature = Some(plan.new_name.clone());
                    s.write(&session_ref.view_dir)?;
                    view::materialise(
                        layout,
                        manifest,
                        feature_novo.as_ref(),
                        s.provider,
                        &session_ref.view_dir,
                    )?;
                }
            }
            Ok(Step::MovePlans)
        }
        Step::MovePlans => {
            if plan.old_feature.name != plan.new_name && fs::is_dir(&plan.old_plan_dir)? {
                fs::rename(&plan.old_plan_dir, &plan.new_plan_dir)?;
            }
            Ok(Step::Finish)
        }
        Step::Finish => {
            if plan.old_feature.name != plan.new_name
                && matches!(
                    fs::read_symlink(&plan.old_dir)?,
                    fs::SymlinkTarget::Target(_)
                )
            {
                fs::remove_path(&plan.old_dir)?;
            }
            Ok(Step::Finish)
        }
    }
}

pub(super) fn undo_step(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    plan: &RenamePlan,
    step: Step,
) -> Result<Step, Failure> {
    match step {
        Step::MovePlans => {
            if plan.old_feature.name != plan.new_name && fs::is_dir(&plan.new_plan_dir)? {
                fs::rename(&plan.new_plan_dir, &plan.old_plan_dir)?;
            }
            Ok(Step::MoveSessions)
        }
        Step::MoveSessions => {
            let current_name = if fs::is_dir(&plan.new_dir)? {
                &plan.new_name
            } else {
                &plan.old_feature.name
            };
            let sessions = lookup::list_feature(layout, current_name)?;
            for session_ref in sessions {
                if let Some(mut s) = session_ref.state {
                    s.feature = Some(plan.old_feature.name.clone());
                    s.write(&session_ref.view_dir)?;
                    view::materialise(
                        layout,
                        manifest,
                        Some(&plan.old_feature),
                        s.provider,
                        &session_ref.view_dir,
                    )?;
                }
            }
            Ok(Step::UpdateChildren)
        }
        Step::UpdateChildren => {
            if plan.old_feature.name != plan.new_name {
                let features_dir = layout.features_dir();
                if fs::is_dir(&features_dir)? {
                    for entry in fs::read_dir(&features_dir)? {
                        if matches!(fs::read_symlink(&entry)?, fs::SymlinkTarget::Target(_)) {
                            continue;
                        }
                        let Some(name) = entry.file_name() else {
                            continue;
                        };
                        let Ok(feature_name) = FeatureName::new(name.to_owned()) else {
                            continue;
                        };
                        if let Some(mut feature) = Feature::read(layout, &feature_name)?
                            && feature.name == feature_name
                            && feature.parent == Some(plan.new_name.clone())
                        {
                            feature.parent = Some(plan.old_feature.name.clone());
                            feature.write(layout)?;
                        }
                    }
                }
            }
            Ok(Step::MoveFeatureDir)
        }
        Step::MoveFeatureDir => {
            if plan.old_feature.name != plan.new_name {
                if matches!(
                    fs::read_symlink(&plan.old_dir)?,
                    fs::SymlinkTarget::Target(_)
                ) {
                    fs::remove_path(&plan.old_dir)?;
                }
                if fs::is_dir(&plan.new_dir)? {
                    fs::rename(&plan.new_dir, &plan.old_dir)?;
                }
            }
            plan.old_feature.write(layout)?;
            Ok(Step::RemoteOps)
        }
        Step::RemoteOps => {
            for r in &plan.repos {
                if r.old_branch != r.new_branch
                    && let (Some(remote), Some(tip)) = (&r.old_remote, &r.old_remote_tip)
                {
                    let bare = layout.repo_bare(&r.repo);
                    if !matches!(
                        git.remote_branch_tip(&bare, remote, r.old_branch.as_str()),
                        Ok(Some(_))
                    ) {
                        git.push(&bare, remote, tip, &format!("refs/heads/{}", r.old_branch))?;
                    }
                    if let Ok(Some(current_new_tip)) =
                        git.remote_branch_tip(&bare, remote, r.new_branch.as_str())
                    {
                        let _ = git.delete_remote_branch(
                            &bare,
                            remote,
                            r.new_branch.as_str(),
                            &current_new_tip,
                        );
                    }
                }
            }
            Ok(Step::MoveWorktrees)
        }
        Step::MoveWorktrees => {
            for r in &plan.repos {
                if r.old_worktree != r.new_worktree {
                    let bare = layout.repo_bare(&r.repo);
                    if fs::is_dir(&r.new_worktree)? {
                        git.move_worktree(&bare, &r.new_worktree, &r.old_worktree)?;
                    }
                }
            }
            Ok(Step::RenameBranches)
        }
        Step::RenameBranches => {
            for r in &plan.repos {
                if r.old_branch != r.new_branch {
                    let bare = layout.repo_bare(&r.repo);
                    if git.revision_commit(&bare, r.new_branch.as_str()).is_ok() {
                        git.rename_branch(&bare, r.new_branch.as_str(), r.old_branch.as_str())?;
                    }
                }
            }
            Ok(Step::Initialize)
        }
        Step::Initialize | Step::Finish => Ok(Step::Initialize),
    }
}
