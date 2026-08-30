//! The repo half of delivery: building each promoted repo's `DeliveryRepo`
//! preview entry, pushing the feature branch, and the dependency ordering that
//! makes the push order (and therefore the fingerprint) well-defined.

use std::collections::BTreeMap;

use camino::Utf8Path;

use crate::domain::feature::{DeliveryAction, DeliveryMode, DeliveryRepo, Feature};
use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction};
use crate::git::{self, TargetState};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::base;
use super::super::pull_requests::existing_pr_url;

pub(crate) fn build_repos(
    git: &impl git::Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    mode: DeliveryMode,
) -> Result<Vec<DeliveryRepo>, Failure> {
    let mut repos = Vec::new();

    for (repo_name, promotion) in &feature.promotions {
        let declared = manifest
            .repos()
            .iter()
            .find(|repo| repo.name() == repo_name)
            .ok_or_else(|| {
                Failure::blocked(
                    "deliver.repo_not_in_manifest",
                    format!(
                        "`{repo_name}` is promoted into `{}` but is no longer in ivar.json",
                        feature.name
                    ),
                )
                .expected("every promoted repo to still be declared in ivar.json")
                .actual(format!("`{repo_name}` does not appear in `repos`"))
                .fix(FixAction::safe(
                    "deliver.restore_manifest",
                    "Restore the repo to ivar.json (or demote it from the feature) before delivering.",
                ))
            })?;

        let bare = layout.repo_bare(repo_name);
        let worktree = layout.repo_worktree(repo_name, &feature.branch);

        match git.target_state(&bare)? {
            TargetState::Repository => {}
            TargetState::Occupied | TargetState::Absent => {
                return Err(Failure::blocked(
                    "repo.bare_not_cloned",
                    format!("`{repo_name}` has no bare clone yet"),
                )
                .expected("the repo to have been cloned by `ivar sync`")
                .actual(format!("`{bare}` does not exist"))
                .fix(FixAction::safe(
                    "repo.sync_first",
                    "Run `ivar sync` to clone the repo, then deliver again.",
                )));
            }
        }

        let mut blockers = Vec::new();

        let branch_exists = git
            .list_branches(&bare)?
            .iter()
            .any(|branch| branch == feature.branch.as_str());
        if !branch_exists {
            blockers.push("branch not materialised; promote the repo first".to_owned());
        }

        // Whether work is unpushed is a fact about the remote, so it is asked
        // of the remote. Every local stand-in for it is wrong here: `deliver`
        // pushes to the manifest URL rather than to a named remote, so git
        // records neither an upstream nor a remote-tracking ref for a push
        // ivar made — and a branch ivar created and delivered, which is the
        // ordinary shape of a new feature, would read as unpushed forever.
        //
        // `ls-remote` is a read, so the preview stays side-effect-free, which
        // is the property that matters; it already reaches the network for the
        // PR action just below.
        if branch_exists {
            let ahead = git.commits_ahead(
                &bare,
                declared.default_branch().as_str(),
                feature.branch.as_str(),
            )?;
            if ahead > 0 {
                match git.remote_branch_tip(&bare, declared.url(), feature.branch.as_str()) {
                    // Only what the remote does not carry is pending. Its tip
                    // is unreadable here when the remote moved on without us,
                    // and then everything beyond the base is the honest count.
                    Ok(Some(tip)) => {
                        let unpushed = git
                            .commits_ahead(&bare, &tip, feature.branch.as_str())
                            .unwrap_or(ahead);
                        if unpushed > 0 {
                            blockers.push(format!("{unpushed} commit(s) not pushed"));
                        }
                    }
                    Ok(None) => blockers.push(format!("{ahead} commit(s) not pushed")),
                    // What the remote holds is unknown, so the doubt is the
                    // report. git's own sentence is left out on purpose: this
                    // text is part of the fingerprint apply is gated on, and
                    // must not vary with the wording of a network error.
                    Err(_) => blockers.push(format!(
                        "{ahead} commit(s) of unconfirmed delivery — the remote did not answer"
                    )),
                }
            }
        }

        let worktree_present = matches!(
            git.target_state(&worktree).unwrap_or(TargetState::Absent),
            TargetState::Repository
        );
        if worktree_present && git.worktree_dirty(&worktree)? {
            blockers.push("worktree has uncommitted changes".to_owned());
        }

        // Predict the delivery action: only GitHub remotes get a PR. A repo
        // on any other host (a local path, a mirror, GitLab) is push-only —
        // `gh` cannot raise a PR there. For GitHub, check the remote: if an
        // open PR already exists for this branch we update it, otherwise we
        // create one. In land mode, the action is landing onto default.
        let action = match mode {
            DeliveryMode::Push => {
                if !crate::infra::github::is_github_https(declared.url()) {
                    DeliveryAction::PushOnly
                } else if existing_pr_url(&bare, feature.branch.as_str()).is_some() {
                    DeliveryAction::UpdatePr
                } else {
                    DeliveryAction::NewPr
                }
            }
            DeliveryMode::Land => DeliveryAction::LandOnDefault,
        };

        let (default_branch, ff_possible) = if mode == DeliveryMode::Land {
            let default_branch = manifest
                .repos()
                .iter()
                .find(|repo| repo.name() == repo_name)
                .map(|repo| repo.default_branch().clone());

            let ff_possible = match &default_branch {
                Some(target) => {
                    Some(git.is_ancestor(&bare, target.as_str(), feature.branch.as_str())?)
                }
                None => Some(false),
            };

            (default_branch, ff_possible)
        } else {
            (None, None)
        };

        repos.push(DeliveryRepo {
            repo: repo_name.clone(),
            local_branch: feature.branch.clone(),
            remote: declared.url().to_owned(),
            push_refspec: format!("{}:refs/heads/{}", feature.branch, feature.branch),
            action,
            base_branch: base::resolve(feature, promotion, declared.default_branch()),
            dependencies: Vec::new(),
            blockers,
            pr_url: None,
            default_branch,
            ff_possible,
        });
    }

    Ok(repos)
}

pub(crate) fn push_repo(
    git: &impl git::Git,
    bare: &Utf8Path,
    repo: &DeliveryRepo,
) -> Result<(), Failure> {
    let branch_exists = git
        .list_branches(bare)?
        .iter()
        .any(|branch| branch == repo.local_branch.as_str());
    if !branch_exists {
        return Err(Failure::blocked(
            "deliver.branch_not_materialised",
            format!(
                "`{}` has no `{}` branch to push",
                repo.repo, repo.local_branch
            ),
        )
        .expected("the feature branch to exist in the repo's bare clone")
        .actual("the branch is not there")
        .fix(FixAction::safe(
            "deliver.promote_first",
            format!(
                "Promote `{}` into the feature first, then deliver again.",
                repo.repo
            ),
        )));
    }

    git.push(
        bare,
        &repo.remote,
        repo.local_branch.as_str(),
        &format!("refs/heads/{}", repo.local_branch),
    )?;
    Ok(())
}

pub(crate) fn order_by_dependencies(repos: &mut Vec<DeliveryRepo>) {
    let mut remaining: BTreeMap<RepoName, usize> = repos
        .iter()
        .map(|repo| (repo.repo.clone(), repo.dependencies.len()))
        .collect();
    let mut ordered = Vec::with_capacity(repos.len());

    while !repos.is_empty() {
        let index = repos.iter().position(|repo| {
            repo.dependencies
                .iter()
                .all(|dep| !remaining.contains_key(dep))
        });
        let Some(index) = index else {
            ordered.append(repos);
            break;
        };
        let repo = repos.remove(index);
        remaining.remove(&repo.repo);
        ordered.push(repo);
    }

    *repos = ordered;
}
