use crate::action::memory::delivery::Emitter;
use crate::domain::delivery_event::{DeliveryEvent, DeliveryScope};
use crate::domain::feature::{DeliveryAction, DeliveryRepo, DraftAction, Feature};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::Failure;
use crate::git::Git;
use crate::store::layout::Layout;

use super::outcome::{LandResult, PullRequestRef, PushResult, RepoCheckResult};

fn pr_action(repo: &DeliveryRepo) -> &'static str {
    match (repo.action, repo.draft) {
        (_, Some(DraftAction::ConvertToDraft)) => "convert-to-draft",
        (DeliveryAction::NewPr, _) => "new",
        _ => "update",
    }
}

fn repo_scope(
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo: &RepoName,
) -> DeliveryScope {
    let head = git
        .head_commit(&layout.repo_worktree(repo, &feature.branch))
        .unwrap_or_default();
    DeliveryScope::for_repo(feature, repo.as_str(), &head)
}

pub(super) fn emit_refused(
    emitter: &Emitter<'_>,
    layout: &Layout,
    feature: &str,
    presented_fingerprint: Option<&str>,
    failure: &Failure,
) {
    let Ok(name) = FeatureName::new(feature) else {
        return;
    };
    let Ok(Some(feature)) = Feature::read(layout, &name) else {
        return;
    };
    emitter.emit(
        DeliveryScope::for_feature(&feature),
        DeliveryEvent::Refused {
            code: failure.code.to_owned(),
            blockers: vec![failure.what.clone()],
            fingerprint: presented_fingerprint.unwrap_or_default().to_owned(),
        },
    );
}

pub(super) fn emit_checks(emitter: &Emitter<'_>, scope: DeliveryScope, check: &RepoCheckResult) {
    let failed = check
        .results
        .iter()
        .filter(|result| !result.success)
        .map(|result| result.command.clone())
        .collect();
    emitter.emit(
        scope,
        DeliveryEvent::Checks {
            passed: check.passed,
            failed,
        },
    );
}

pub(super) fn emit_repo_pushed(
    emitter: &Emitter<'_>,
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo: &DeliveryRepo,
    check: &RepoCheckResult,
    push: &PushResult,
) {
    let scope = repo_scope(git, layout, feature, &repo.repo);
    emit_checks(emitter, scope.clone(), check);
    emitter.emit(
        scope,
        DeliveryEvent::Pushed {
            ok: push.ok,
            detail: push.detail.clone(),
            base: repo.base_branch.to_string(),
        },
    );
}

pub(super) fn emit_pr_opened(
    emitter: &Emitter<'_>,
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    repo: &DeliveryRepo,
    pr: &PullRequestRef,
) {
    emitter.emit(
        repo_scope(git, layout, feature, &repo.repo),
        DeliveryEvent::Pr {
            action: pr_action(repo).into(),
            number: pr.number,
            url: pr.url.clone(),
            draft: pr.draft,
        },
    );
}

pub(super) fn emit_land_outcome(
    emitter: &Emitter<'_>,
    git: &impl Git,
    layout: &Layout,
    feature: &Feature,
    land: &[LandResult],
) {
    for result in land {
        emitter.emit(
            repo_scope(git, layout, feature, &result.repo),
            DeliveryEvent::Landed {
                merged: result.merged,
                pushed: result.pushed,
                detail: result.detail.clone(),
            },
        );
    }
}
