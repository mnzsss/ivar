//! Non-land apply execution: ordered verification checks, best-effort branch push, PR creation and linking.

use std::collections::BTreeMap;

use crate::action::feature::pull_requests::{
    PullRequest, convert_pull_request_to_draft, create_pull_request, edit_pull_request,
    existing_pr, link_sibling_prs,
};
use crate::action::feature::verification;
use crate::action::memory::delivery::Emitter;
use crate::domain::feature::{DeliveryAction, DeliveryPreview, DraftAction, Feature};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Report, Warning};
use crate::git::Git;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::events;
use super::outcome::{DeliverOutcome, PullRequestRef, PushResult, RepoCheckResult};
use super::repos::{base_failure, push_repo};

/// Execute non-land delivery apply: run root repo verification checks, push feature branches best-effort, and handle PRs.
#[cfg(test)]
pub(crate) fn execute(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    feature: &Feature,
    preview: DeliveryPreview,
) -> Result<Report<DeliverOutcome>, Failure> {
    execute_with(git, layout, manifest, feature_name, feature, preview, None)
}

/// [`execute`], recording each repo's `checks` and `pushed` events as that
/// repo is pushed and its `pr` event once the PR exists, so a later repo's
/// failure cannot lose what an earlier one already did.
pub(super) fn execute_with(
    git: &impl Git,
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    feature: &Feature,
    mut preview: DeliveryPreview,
    emitter: Option<&Emitter<'_>>,
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
        if let Some(emitter) = emitter {
            events::emit_repo_pushed(emitter, git, layout, feature, repo, &check_result, &push);
        }
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
                    let pr_ref = PullRequestRef {
                        number: pr.number,
                        url: pr.url.clone(),
                        draft: pr.is_draft,
                    };
                    if let (Some(emitter), Some(delivery)) = (
                        emitter,
                        preview.repos.iter().find(|repo| repo.repo == repo_name),
                    ) {
                        events::emit_pr_opened(emitter, git, layout, feature, delivery, &pr_ref);
                    }
                    push.pr = Some(pr_ref);
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

    if !pr_url_map.is_empty()
        && let Err(failure) = remember_pull_requests(layout, feature_name, &pr_url_map)
    {
        warnings.push(Warning::new(
            "deliver.pr_url_not_saved",
            feature_name.as_str(),
            failure.what,
        ));
    }

    // -- Phase 3: link sibling PRs (second pass — URLs only known after phase 2)
    let pr_urls: Vec<String> = pr_url_map.into_values().collect();
    if !pr_urls.is_empty() && links_siblings(&preview, feature) {
        link_sibling_prs(&pr_urls);
    }

    Ok(Report::with_warnings(
        DeliverOutcome {
            root: layout.root().to_path_buf(),
            preview,
            blockers: Vec::new(),
            apply_command: None,
            pushes,
            land: Vec::new(),
            checks,
        },
        warnings,
    ))
}

/// Stored on each promotion so `feature status` can later ask the forge
/// whether the delivery merged.
fn remember_pull_requests(
    layout: &Layout,
    feature_name: &FeatureName,
    pr_urls: &BTreeMap<RepoName, String>,
) -> Result<(), Failure> {
    Feature::update(layout, feature_name, |feature| {
        for (repo, url) in pr_urls {
            if let Some(promotion) = feature.promotions.get_mut(repo) {
                promotion.pr_url = Some(url.clone());
            }
        }
        Ok(())
    })
}

/// A partial delivery cannot see the PRs of the repos it did not select, so
/// it leaves the Sibling PRs comment of the full delivery intact.
pub(crate) fn links_siblings(preview: &DeliveryPreview, feature: &Feature) -> bool {
    preview.repos.len() == feature.promotions.len()
}

/// How a repo's branch reached the remote.
enum Pushed {
    /// An ordinary push: the remote's branch was an ancestor of ours.
    Plain,
    /// The remote's branch was replaced, with a lease on this tip — the one
    /// the last delivery recorded.
    Replaced(String),
}

/// Push the repo's branch; when git refuses it as non-fast-forward and the
/// remote branch is exactly the tip ivar's last delivery pushed, replace it
/// with a lease on that tip.
///
/// This is how the branch looks after `ivar feature rebase`: the remote holds
/// ivar's own earlier push, which the rewritten branch no longer contains.
/// Any other remote tip — a commit somebody else pushed, or a delivery that
/// recorded nothing — is not ours to replace, and the original refusal is
/// returned untouched. So is a lease that fails, the remote having moved in
/// the instant since it was read: the recovery command re-reads the tip.
fn push_or_replace_own(
    git: &impl Git,
    bare: &camino::Utf8Path,
    repo: &crate::domain::feature::DeliveryRepo,
    delivered: Option<&str>,
) -> Result<Pushed, Failure> {
    let refusal = match push_repo(git, bare, repo) {
        Ok(()) => return Ok(Pushed::Plain),
        Err(failure) => failure,
    };
    let Some(delivered) = delivered.filter(|_| rejected_as_non_fast_forward(&refusal)) else {
        return Err(refusal);
    };
    let remote_tip = git
        .remote_branch_tip(bare, &repo.remote, repo.local_branch.as_str())
        .ok()
        .flatten();
    if remote_tip.as_deref() != Some(delivered) {
        return Err(refusal);
    }
    match git.push_with_lease(
        bare,
        &repo.remote,
        repo.local_branch.as_str(),
        &format!("refs/heads/{}", repo.local_branch),
        delivered,
    ) {
        Ok(()) => Ok(Pushed::Replaced(delivered.to_owned())),
        Err(_) => Err(refusal),
    }
}

/// Stored on the promotion so a later delivery can tell the remote's tip is
/// ivar's own push. Written on the push, not on the PR: a branch pushed
/// whose PR then failed must still be recognised.
fn remember_delivered_sha(
    layout: &Layout,
    feature_name: &FeatureName,
    repo: &crate::domain::feature::DeliveryRepo,
    sha: &str,
) -> Result<(), Failure> {
    Feature::update(layout, feature_name, |feature| {
        if let Some(promotion) = feature.promotions.get_mut(&repo.repo) {
            promotion.delivered_sha = Some(sha.to_owned());
        }
        Ok(())
    })
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
    let delivered = feature
        .promotions
        .get(&repo.repo)
        .and_then(|promotion| promotion.delivered_sha.as_deref());
    // The tip about to be pushed, read before the push so what is recorded
    // is what was sent, whatever a human does to the branch meanwhile.
    let pushing = git
        .revision_commit(&bare, &format!("refs/heads/{}", repo.local_branch))
        .ok();
    let push = match push_or_replace_own(git, &bare, repo, delivered) {
        Ok(pushed) => {
            match &pushing {
                Some(sha) => {
                    if let Err(failure) = remember_delivered_sha(layout, &feature.name, repo, sha) {
                        warnings.push(Warning::new(
                            "deliver.delivered_sha_not_saved",
                            repo.repo.as_str(),
                            failure.what,
                        ));
                    }
                }
                None => warnings.push(Warning::new(
                    "deliver.delivered_sha_not_saved",
                    repo.repo.as_str(),
                    "the pushed tip could not be read, so a later delivery cannot recognise this push",
                )),
            }
            PushResult {
                repo: repo.repo.clone(),
                ok: true,
                detail: match pushed {
                    Pushed::Plain => None,
                    Pushed::Replaced(sha) => Some(format!(
                        "force-pushed with lease: replaced `{}` at {sha}, the tip the last delivery pushed",
                        repo.remote_branch(repo.local_branch.as_str()),
                    )),
                },
                pr: None,
                fix: None,
            }
        }
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
    if !pushes.iter().any(|push| push.repo == repo.repo && push.ok) {
        return None;
    }

    let bare = layout.repo_bare(&repo.repo);

    if let Some(failure) = base_failure(git, manifest, &bare, feature_name, repo) {
        warnings.push(Warning::new(
            failure.code,
            repo.repo.as_str(),
            failure.what.clone(),
        ));
        if let Some(push) = pushes.iter_mut().find(|push| push.repo == repo.repo) {
            push.detail = Some(match push.detail.take() {
                Some(earlier) => format!("no pull request: {}; {earlier}", failure.what),
                None => format!("no pull request: {}", failure.what),
            });
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
        DeliveryAction::UpdatePr => match existing_pr(bare, repo.local_branch.as_str()) {
            Err(failure) => (Err(failure), false),
            Ok(None) => (
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
            Ok(Some(pr)) => (
                edit_pull_request(
                    bare,
                    &pr.url,
                    repo.pr_title.as_deref(),
                    repo.pr_body.as_deref(),
                )
                .map(|_| pr),
                true,
            ),
        },
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
/// ivar force-pushes on its own only over the tip its last delivery recorded
/// (see [`push_or_replace_own`]). Anything else on the remote can be work that
/// is only there, so replacing it stays a human's decision.
fn force_with_lease_fix(
    git: &impl Git,
    bare: &camino::Utf8Path,
    repo: &crate::domain::feature::DeliveryRepo,
) -> FixAction {
    let branch = repo.local_branch.as_str();
    let remote_branch = repo.remote_branch(branch);
    // Unprefixed: the short name, exactly as before. Prefixed: the full ref,
    // so the lease guards the same ref the push replaces.
    let lease_ref = if repo.ref_prefix.is_some() {
        format!("refs/heads/{remote_branch}")
    } else {
        branch.to_owned()
    };
    let lease = git
        .remote_branch_tip(bare, &repo.remote, branch)
        .ok()
        .flatten()
        .map_or_else(
            || format!("--force-with-lease={lease_ref}"),
            |tip| format!("--force-with-lease={lease_ref}:{tip}"),
        );
    FixAction::unsafe_(
        "deliver.force_with_lease",
        format!(
            "Review what `{}` already holds on `{remote_branch}` — this replaces it. Then push it by hand.",
            repo.remote
        ),
    )
    .command(format!(
        "git --git-dir {bare} push {lease} {} {branch}:refs/heads/{remote_branch}",
        repo.remote
    ))
}
