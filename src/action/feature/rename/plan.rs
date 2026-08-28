
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::Failure;
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::infra::fs;


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoRenamePlan {
    pub repo: RepoName,
    pub old_worktree: Utf8PathBuf,
    pub new_worktree: Utf8PathBuf,
    pub old_branch: BranchName,
    pub new_branch: BranchName,
    pub old_remote: Option<String>,
    pub old_remote_tip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlan {
    pub old_feature: Feature,
    pub new_name: FeatureName,
    pub new_branch: BranchName,
    pub old_dir: Utf8PathBuf,
    pub new_dir: Utf8PathBuf,
    pub old_plan_dir: Utf8PathBuf,
    pub new_plan_dir: Utf8PathBuf,
    pub repos: Vec<RepoRenamePlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    pub scope: String,
    pub subject: String,
    pub explanation: String,
}

use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::Failure;
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::infra::{fs, json};
use crate::action::feature::pull_requests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoRenamePlan {
    pub repo: RepoName,
    pub old_worktree: Utf8PathBuf,
    pub new_worktree: Utf8PathBuf,
    pub old_branch: BranchName,
    pub new_branch: BranchName,
    pub old_remote: Option<String>,
    pub old_remote_tip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlan {
    pub old_feature: Feature,
    pub new_name: FeatureName,
    pub new_branch: BranchName,
    pub old_dir: Utf8PathBuf,
    pub new_dir: Utf8PathBuf,
    pub old_plan_dir: Utf8PathBuf,
    pub new_plan_dir: Utf8PathBuf,
    pub repos: Vec<RepoRenamePlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blocker {
    pub scope: String,
    pub subject: String,
    pub explanation: String,
}

pub fn build(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
    source: &Feature,
    new_name: FeatureName,
    new_branch_input: Option<BranchName>,
) -> Result<(RenamePlan, Vec<Blocker>), Failure> {
    let mut blockers = Vec::new();
    let new_branch = new_branch_input.clone().unwrap_or_else(|| source.branch.clone());

    // R-FEATURE-COLLISIONS: occupied `.ivar/features/<new-name>`
    let new_dir = layout.feature_dir(&new_name);
    if fs::is_dir(&new_dir)? {
        blockers.push(Blocker {
            scope: "feature".to_string(),
            subject: new_name.to_string(),
            explanation: format!("Feature directory `{new_dir}` already exists."),
        });
    }

    // R-FEATURE-COLLISIONS: occupied `plans/<new-name>`
    let old_plan_dir = layout.plan_dir(&source.name);
    let new_plan_dir = layout.plan_dir(&new_name);
    if fs::is_dir(&new_plan_dir)? {
        blockers.push(Blocker {
            scope: "plan".to_string(),
            subject: new_name.to_string(),
            explanation: format!("Plan directory `{new_plan_dir}` already exists."),
        });
    }

    let mut repo_plans = Vec::new();

    for (repo_name, _promotion) in &source.promotions {
        let repo = match manifest.repo(repo_name) {
            Some(r) => r,
            None => {
                blockers.push(Blocker {
                    scope: "repository".to_string(),
                    subject: repo_name.to_string(),
                    explanation: "Repository not found in manifest.".to_string(),
                });
                continue;
            }
        };

        // R-REPO-PREFLIGHT: resolve current worktree, check conflict
        let old_worktree = layout.repo_worktree(repo_name, &source.branch);
        let new_worktree = layout.repo_worktree(repo_name, &new_branch);

        if let Err(_) = git.worktree_git_dir(&old_worktree) {
            blockers.push(Blocker {
                scope: "repository".to_string(),
                subject: repo_name.to_string(),
                explanation: format!("Worktree `{old_worktree}` is not registered by Git."),
            });
        }
        
        if fs::is_dir(&new_worktree)? {
             blockers.push(Blocker {
                scope: "repository".to_string(),
                subject: repo_name.to_string(),
                explanation: format!("Target worktree `{new_worktree}` already exists."),
            });
        }

        // R-REMOTE-PREFLIGHT
        let bare = layout.repo_bare(repo_name);
        let remote = repo.url();
        let old_remote_tip = git.remote_branch_tip(&bare, remote, source.branch.as_str())?;

        // R-OPEN-PRS
        match pull_requests::find_pull_request(&bare, source.branch.as_str(), "open") {
            Ok(Some(pr)) => {
                blockers.push(Blocker {
                    scope: "pull-request".to_string(),
                    subject: repo_name.to_string(),
                    explanation: format!("Open PR #{} (URL: {}) targets old branch: {}. Close it first.", pr.number, pr.url, source.branch),
                });
            }
            Ok(None) => {}
            Err(e) => {
                 blockers.push(Blocker {
                    scope: "pull-request".to_string(),
                    subject: repo_name.to_string(),
                    explanation: format!("Failed to check open PRs: {}", e),
                });
            }
        }

        repo_plans.push(RepoRenamePlan {
            repo: repo_name.clone(),
            old_worktree,
            new_worktree,
            old_branch: source.branch.clone(),
            new_branch: new_branch.clone(),
            old_remote: Some(remote.to_string()),
            old_remote_tip,
        });
    }

    let plan = RenamePlan {
        old_feature: source.clone(),
        new_name,
        new_branch,
        old_dir: layout.feature_dir(&source.name),
        new_dir,
        old_plan_dir,
        new_plan_dir,
        repos: repo_plans,
    };

    Ok((plan, blockers))
}
 new_name.to_string(),
            explanation: format!("Feature directory `{new_dir}` already exists."),
        });
    }

    // R-REPO-PREFLIGHT: check registered worktree
    for (repo_name, _promotion) in &source.promotions {
        let worktree = layout.repo_worktree(repo_name, &source.branch);
        if let Err(_) = git.worktree_git_dir(&worktree) {
            blockers.push(Blocker {
                scope: "repository".to_string(),
                subject: repo_name.to_string(),
                explanation: format!("Worktree `{worktree}` is not registered by Git."),
            });
        }
    }

    // R-OPEN-PRS: check open PRs
    for (repo_name, _promotion) in &source.promotions {
        let bare = layout.repo_bare(repo_name);
        match crate::action::feature::pull_requests::find_pull_request(&bare, source.branch.as_str(), "open") {
            Ok(Some(pr)) => {
                blockers.push(Blocker {
                    scope: "pull-request".to_string(),
                    subject: repo_name.to_string(),
                    explanation: format!("Open PR #{} targets old branch: {}. Close it first.", pr.number, pr.url),
                });
            }
            Ok(None) => {}
            Err(_) => {}
        }
    }

    let plan = RenamePlan {
        old_feature: source.clone(),
        new_name: new_name.clone(),
        new_branch: new_branch.unwrap_or_else(|| source.branch.clone()),
        old_dir: layout.feature_dir(&source.name),
        new_dir: new_dir.clone(),
        old_plan_dir: layout.plan_dir(&source.name),
        new_plan_dir: layout.plan_dir(&new_name),
        repos: vec![],
    };


    Ok((plan, blockers))
}


