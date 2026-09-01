//! Non-land apply execution: ordered verification checks, best-effort branch push, PR creation and linking.

use std::collections::BTreeMap;

use crate::action::feature::pull_requests::{
    create_pull_request, edit_pull_request, existing_pr_url, link_sibling_prs,
};
use crate::action::feature::verification;
use crate::domain::feature::{DeliveryAction, DeliveryPreview, Feature};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, Report, Warning};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::outcome::{DeliverOutcome, PushResult, RepoCheckResult};
use super::repos::push_repo;

/// Execute non-land delivery apply: run root repo verification checks, push feature branches best-effort, and handle PRs.
pub(super) fn execute(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    feature: &Feature,
    mut preview: DeliveryPreview,
) -> Result<Report<DeliverOutcome>, Failure> {
    let mut pushes = Vec::new();
    let mut checks = Vec::new();
    let mut warnings = Vec::new();

    // -- Phase 1: run each root repo's ordered checks, then push best-effort --
    // A repo whose checks fail is not pushed — its work did not verify — while
    // the rest of the batch continues. The results are machine-visible on the outcome.
    for repo in &preview.repos {
        let worktree = layout.repo_worktree(&repo.repo, &feature.branch);
        let repo_checks = verification::checks_for(manifest, &repo.repo);
        let run = verification::run(&repo_checks, &worktree)?;
        let passed = run.results.iter().all(|result| result.success);
        checks.push(RepoCheckResult {
            repo: repo.repo.clone(),
            passed,
            results: run.results,
        });
        if !passed {
            warnings.push(Warning::new(
                "deliver.checks_failed",
                repo.repo.as_str(),
                "root checks failed; this repo was not pushed",
            ));
            pushes.push(PushResult {
                repo: repo.repo.clone(),
                ok: false,
                detail: Some("root checks failed".to_owned()),
            });
            continue;
        }

        let bare = layout.repo_bare(&repo.repo);
        match push_repo(git, &bare, repo) {
            Ok(()) => pushes.push(PushResult {
                repo: repo.repo.clone(),
                ok: true,
                detail: None,
            }),
            Err(failure) => {
                let detail = failure.what.clone();
                warnings.push(Warning::new(
                    "deliver.push_failed",
                    repo.repo.as_str(),
                    detail.clone(),
                ));
                pushes.push(PushResult {
                    repo: repo.repo.clone(),
                    ok: false,
                    detail: Some(detail),
                });
            }
        }
    }

    // -- Phase 2: create PRs for repos that need them -------------------------
    let mut pr_url_map: BTreeMap<RepoName, String> = BTreeMap::new();
    let mut pr_results: Vec<(RepoName, Result<String, Failure>)> = Vec::new();
    for repo in &preview.repos {
        if matches!(
            repo.action,
            DeliveryAction::PushOnly | DeliveryAction::LandOnDefault
        ) {
            continue;
        }

        let bare = layout.repo_bare(&repo.repo);

        // The base must still support delivering onto it before a PR is
        // opened or updated against it: a base gone from the remote, or one
        // this branch has drifted off of, would make the PR's diff wrong.
        // Refused per repo — the rest of the batch is unaffected — and never
        // added to `blockers`, which is informational only.
        let default_branch = manifest
            .repos()
            .iter()
            .find(|manifest_repo| manifest_repo.name() == &repo.repo)
            .map(|manifest_repo| manifest_repo.default_branch().clone());
        if let Some(default_branch) = default_branch {
            let remote_tip = git
                .remote_branch_tip(&bare, &repo.remote, repo.base_branch.as_str())
                .map_err(|_| ());
            let secondary = match &remote_tip {
                // Ignored by `check_base` when the remote did not answer —
                // no point spending a local read on it.
                Err(()) => Ok(false),
                Ok(None) => git
                    .is_ancestor(&bare, repo.base_branch.as_str(), default_branch.as_str())
                    .map_err(|_| ()),
                // Against the remote's own tip, not the local branch name:
                // `ivar sync` never re-fetches a non-default branch, so a
                // local `base_branch` ref can be stale — still an ancestor
                // of the local branch even though the remote has moved on.
                // A tip this bare clone never fetched is itself the answer
                // (`is_ancestor` refuses, `check_base` reads that as moved).
                Ok(Some(tip)) => git
                    .is_ancestor(&bare, tip, repo.local_branch.as_str())
                    .map_err(|_| ()),
            };
            if let Some(failure) = repo.check_base(remote_tip, secondary, &default_branch) {
                warnings.push(Warning::new(
                    failure.code,
                    repo.repo.as_str(),
                    failure.what.clone(),
                ));
                continue;
            }
        }

        // A branch that already has a PR was updated by the push above — `gh pr
        // create` would only refuse it as a duplicate. Its URL is still part of
        // the report, and `gh pr list` is the only place it comes from.
        let result = match repo.action {
            DeliveryAction::UpdatePr => {
                // Try to find existing PR; if it exists, do a partial edit; otherwise create new.
                existing_pr_url(&bare, repo.local_branch.as_str()).map_or_else(
                    || {
                        create_pull_request(
                            &bare,
                            &repo.local_branch,
                            &repo.base_branch,
                            feature_name,
                            repo.pr_title.as_deref(),
                            repo.pr_body.as_deref(),
                        )
                        .map(|pr| pr.url)
                    },
                    |url| {
                        // PR exists — do a safe partial edit (only supplied fields change).
                        edit_pull_request(
                            &bare,
                            &url,
                            repo.pr_title.as_deref(),
                            repo.pr_body.as_deref(),
                        )
                        .map(|_| url)
                    },
                )
            }
            DeliveryAction::NewPr => create_pull_request(
                &bare,
                &repo.local_branch,
                &repo.base_branch,
                feature_name,
                repo.pr_title.as_deref(),
                repo.pr_body.as_deref(),
            )
            .map(|pr| pr.url),
            DeliveryAction::PushOnly | DeliveryAction::LandOnDefault => unreachable!(),
        };
        pr_results.push((repo.repo.clone(), result));
    }

    for (repo_name, result) in pr_results {
        match result {
            Ok(url) => {
                pr_url_map.insert(repo_name.clone(), url);
            }
            Err(failure) => {
                let detail = failure.what.clone();
                warnings.push(Warning::new(
                    "deliver.pr_create_failed",
                    repo_name.as_str(),
                    detail.clone(),
                ));
            }
        }
    }

    // Record PR URLs on the preview repos so they round-trip through JSON.
    for repo in &mut preview.repos {
        if let Some(url) = pr_url_map.get(&repo.repo) {
            repo.pr_url = Some(url.clone());
        }
    }

    // -- Phase 3: link sibling PRs (second pass — URLs only known after phase 2)
    let pr_urls: Vec<String> = pr_url_map.into_values().collect();
    if !pr_urls.is_empty() {
        link_sibling_prs(&pr_urls);
    }

    Ok(Report::with_warnings(
        DeliverOutcome {
            root: layout.root().to_path_buf(),
            preview,
            pushes,
            land: Vec::new(),
            checks,
        },
        warnings,
    ))
}
