//! Non-land apply execution: ordered verification checks, best-effort branch push, PR creation and linking.

use std::collections::BTreeMap;

use crate::action::feature::pull_requests::{
    PullRequest, convert_pull_request_to_draft, create_pull_request, edit_pull_request,
    existing_pr, link_sibling_prs,
};
use crate::action::feature::verification;
use crate::domain::feature::{DeliveryAction, DeliveryPreview, DraftAction, Feature};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Report, Warning};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::outcome::{DeliverOutcome, PullRequestRef, PushResult, RepoCheckResult};
use super::repos::push_repo;

/// Execute non-land delivery apply: run root repo verification checks, push feature branches best-effort, and handle PRs.
pub(crate) fn execute(
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
        let (push, check_result, repo_warnings) =
            check_and_push_one(git, layout, manifest, feature, repo)?;
        pushes.push(push);
        checks.push(check_result);
        warnings.extend(repo_warnings);
    }

    // -- Phase 2: create PRs for repos that need them -------------------------
    let mut pr_results: Vec<(RepoName, Result<PullRequest, Failure>)> = Vec::new();
    for repo in &preview.repos {
        if let Some(entry) = create_pr_for_repo(
            git,
            manifest,
            layout,
            feature_name,
            repo,
            &mut pushes,
            &mut warnings,
        ) {
            pr_results.push(entry);
        }
    }

    let mut pr_url_map: BTreeMap<RepoName, String> = BTreeMap::new();
    for (repo_name, result) in pr_results {
        match result {
            Ok(pr) => {
                if let Some(push) = pushes.iter_mut().find(|push| push.repo == repo_name) {
                    push.pr = Some(PullRequestRef {
                        number: pr.number,
                        url: pr.url.clone(),
                        draft: pr.is_draft,
                    });
                }
                pr_url_map.insert(repo_name, pr.url);
            }
            Err(failure) => {
                warnings.push(Warning::new(
                    "deliver.pr_create_failed",
                    repo_name.as_str(),
                    failure.what.clone(),
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
            apply_command: None,
            pushes,
            land: Vec::new(),
            checks,
        },
        warnings,
    ))
}

fn check_and_push_one(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
    repo: &crate::domain::feature::DeliveryRepo,
) -> Result<(PushResult, RepoCheckResult, Vec<Warning>), Failure> {
    let mut warnings = Vec::new();
    let worktree = layout.repo_worktree(&repo.repo, &feature.branch);
    let repo_checks = verification::checks_for(manifest, &repo.repo);
    let run = verification::run(&repo_checks, &worktree)?;
    let passed = run.results.iter().all(|result| result.success);
    let check_result = RepoCheckResult {
        repo: repo.repo.clone(),
        passed,
        results: run.results,
    };
    if !passed {
        warnings.push(Warning::new(
            "deliver.checks_failed",
            repo.repo.as_str(),
            "root checks failed; this repo was not pushed",
        ));
        let push = PushResult {
            repo: repo.repo.clone(),
            ok: false,
            detail: Some("root checks failed".to_owned()),
            pr: None,
            fix: None,
        };
        return Ok((push, check_result, warnings));
    }

    let bare = layout.repo_bare(&repo.repo);
    let push = match push_repo(git, &bare, repo) {
        Ok(()) => PushResult {
            repo: repo.repo.clone(),
            ok: true,
            detail: None,
            pr: None,
            fix: None,
        },
        Err(failure) => {
            let (detail, fix) = if rejected_as_non_fast_forward(&failure) {
                (
                    "push rejected: the remote branch carries commits this branch does not"
                        .to_owned(),
                    Some(force_with_lease_fix(git, &bare, repo)),
                )
            } else {
                (failure.what, None)
            };
            warnings.push(Warning::new(
                "deliver.push_failed",
                repo.repo.as_str(),
                detail.clone(),
            ));
            PushResult {
                repo: repo.repo.clone(),
                ok: false,
                detail: Some(detail),
                pr: None,
                fix,
            }
        }
    };
    Ok((push, check_result, warnings))
}

#[allow(clippy::too_many_arguments)]
fn create_pr_for_repo(
    git: &impl Git,
    manifest: &Manifest,
    layout: &Layout,
    feature_name: &FeatureName,
    repo: &crate::domain::feature::DeliveryRepo,
    pushes: &mut [PushResult],
    warnings: &mut Vec<Warning>,
) -> Option<(RepoName, Result<PullRequest, Failure>)> {
    if matches!(
        repo.action,
        DeliveryAction::PushOnly | DeliveryAction::LandOnDefault
    ) {
        return None;
    }

    let bare = layout.repo_bare(&repo.repo);

    if let Some(failure) = check_pr_base(git, manifest, &bare, repo) {
        warnings.push(Warning::new(
            failure.code,
            repo.repo.as_str(),
            failure.what.clone(),
        ));
        if let Some(push) = pushes.iter_mut().find(|push| push.repo == repo.repo) {
            push.detail = Some(format!("no pull request: {}", failure.what));
            push.fix = failure.fix_actions.first().cloned();
        }
        return None;
    }

    let (mut result, should_convert) = open_or_update_pr(git, &bare, feature_name, repo);

    // Convert only an existing PR. A planned conversion whose PR vanished
    // is recreated as draft above, so it needs no follow-up transition.
    if repo.draft == Some(DraftAction::ConvertToDraft)
        && should_convert
        && let Ok(pr) = &mut result
    {
        match convert_pull_request_to_draft(&bare, &pr.url) {
            Ok(()) => pr.is_draft = true,
            Err(failure) => warnings.push(Warning::new(
                "deliver.pr_draft_conversion_failed",
                repo.repo.as_str(),
                format!("{}: {}", failure.code, failure.what),
            )),
        }
    }

    Some((repo.repo.clone(), result))
}

/// The base must still support delivering onto it before a PR is opened or
/// updated against it: a base gone from the remote, or one this branch has
/// drifted off of, would make the PR's diff wrong. `None` means the check
/// passed, or there was nothing to check.
fn check_pr_base(
    git: &impl Git,
    manifest: &Manifest,
    bare: &camino::Utf8Path,
    repo: &crate::domain::feature::DeliveryRepo,
) -> Option<Failure> {
    let default_branch = manifest
        .repos()
        .iter()
        .find(|manifest_repo| manifest_repo.name() == &repo.repo)
        .map(|manifest_repo| manifest_repo.default_branch().clone())?;

    let remote_tip = git
        .remote_branch_tip(bare, &repo.remote, repo.base_branch.as_str())
        .map_err(|_| ());
    let secondary = match &remote_tip {
        // Ignored by `check_base` when the remote did not answer —
        // no point spending a local read on it.
        Err(()) => Ok(false),
        Ok(None) => git
            .is_ancestor(bare, repo.base_branch.as_str(), default_branch.as_str())
            .map_err(|_| ()),
        // Against the remote's own tip, not the local branch name:
        // `ivar sync` never re-fetches a non-default branch, so a
        // local `base_branch` ref can be stale — still an ancestor
        // of the local branch even though the remote has moved on.
        // A tip this bare clone never fetched is itself the answer
        // (`is_ancestor` refuses, `check_base` reads that as moved).
        Ok(Some(tip)) => git
            .is_ancestor(bare, tip, repo.local_branch.as_str())
            .map_err(|_| ()),
    };
    repo.check_base(&remote_tip, secondary, &default_branch)
}

/// Create or update the PR for a repo. A branch that already has a PR was
/// updated by the push above — `gh pr create` would only refuse it as a
/// duplicate. Its URL is still part of the report, and `gh pr list` is the
/// only place it comes from. The returned `bool` is `should_convert`.
fn open_or_update_pr(
    _git: &impl Git,
    bare: &camino::Utf8Path,
    feature_name: &FeatureName,
    repo: &crate::domain::feature::DeliveryRepo,
) -> (Result<PullRequest, Failure>, bool) {
    let want_draft = repo.draft.is_some();
    match repo.action {
        DeliveryAction::UpdatePr => {
            // Try to find existing PR; if it exists, do a partial edit; otherwise create new.
            existing_pr(bare, repo.local_branch.as_str()).map_or_else(
                || {
                    (
                        create_pull_request(
                            bare,
                            &repo.local_branch,
                            &repo.base_branch,
                            feature_name,
                            repo.pr_title.as_deref(),
                            repo.pr_body.as_deref(),
                            want_draft,
                        ),
                        false,
                    )
                },
                |pr| {
                    // PR exists — do a safe partial edit (only supplied fields change).
                    (
                        edit_pull_request(
                            bare,
                            &pr.url,
                            repo.pr_title.as_deref(),
                            repo.pr_body.as_deref(),
                        )
                        .map(|_| pr),
                        true,
                    )
                },
            )
        }
        DeliveryAction::NewPr => (
            create_pull_request(
                bare,
                &repo.local_branch,
                &repo.base_branch,
                feature_name,
                repo.pr_title.as_deref(),
                repo.pr_body.as_deref(),
                want_draft,
            ),
            false,
        ),
        DeliveryAction::PushOnly | DeliveryAction::LandOnDefault => unreachable!(),
    }
}

/// Whether git refused the push because the remote branch has moved on.
///
/// Matched on git's own wording: there is no exit code or porcelain that says
/// this, and the sentence is what an operator would otherwise have to read.
fn rejected_as_non_fast_forward(failure: &Failure) -> bool {
    let text = failure.actual.as_deref().unwrap_or(&failure.what);
    text.contains("non-fast-forward")
        || text.contains("fetch first")
        || text.contains("Updates were rejected")
}

/// The recovery for a rejected push, as a command a human runs themselves.
///
/// ivar never force-pushes on its own: replacing a remote branch can drop work
/// that is only there.
fn force_with_lease_fix(
    git: &impl Git,
    bare: &camino::Utf8Path,
    repo: &crate::domain::feature::DeliveryRepo,
) -> FixAction {
    let branch = repo.local_branch.as_str();
    let lease = git
        .remote_branch_tip(bare, &repo.remote, branch)
        .ok()
        .flatten()
        .map_or_else(
            || format!("--force-with-lease={branch}"),
            |tip| format!("--force-with-lease={branch}:{tip}"),
        );
    FixAction::unsafe_(
        "deliver.force_with_lease",
        format!(
            "Review what `{}` already holds on `{branch}` — this replaces it. Then push it by hand.",
            repo.remote
        ),
    )
    .command(format!(
        "git --git-dir {bare} push {lease} {} {branch}:refs/heads/{branch}",
        repo.remote
    ))
}
