use camino::Utf8Path;

use super::{GhPrRecord, OPEN, PullRequest, capture, parse_gh};
use crate::error::Failure;
use crate::infra::proc;

/// Find the pull request whose head is `branch` and whose state is `state`
/// (`open` or `all`). `Ok(None)` when there is none; a `gh` failure is a
/// strict error, never a silent "no PR".
pub(crate) fn find_pull_request(
    git_dir: &Utf8Path,
    head: &str,
    state: &str,
) -> Result<Option<PullRequest>, Failure> {
    Ok(list_pull_requests(git_dir, head, state)?.into_iter().next())
}

/// Every pull request whose head is `branch` and whose state is `state`. A
/// branch can carry more than one record once it has been reused, so callers
/// that must identify a specific PR pick from here rather than trusting order.
pub(crate) fn list_pull_requests(
    git_dir: &Utf8Path,
    head: &str,
    state: &str,
) -> Result<Vec<PullRequest>, Failure> {
    let output = capture(
        &proc::Command::new("gh")
            .args([
                "pr",
                "list",
                "--head",
                head,
                "--state",
                state,
                "--json",
                "url,number,state,mergeCommit,headRefOid,isDraft,mergedAt",
            ])
            .cwd(git_dir),
        "pr list",
    )?;
    let records: Vec<GhPrRecord> = parse_gh(&output, "pr list")?;
    Ok(records.into_iter().map(PullRequest::from).collect())
}

/// The open pull request for `branch`, when there is one.
pub(crate) fn existing_pr(
    git_dir: &Utf8Path,
    branch: &str,
) -> Result<Option<PullRequest>, Failure> {
    find_pull_request(git_dir, branch, "open")
}

/// What one `gh pr list --state all` says about a head branch: the PR that is
/// open, and the ones that merged. Made in a single call so a decision built
/// from both — the delivery action and the draft decision, and whether the
/// branch was already merged — rests on one fact.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BranchPullRequests {
    /// The open PR, when there is one.
    pub open: Option<PullRequest>,
    /// Every merged PR, newest (highest number) first. A `CLOSED` PR that
    /// never merged is not kept: it blocks nothing.
    pub merged: Vec<PullRequest>,
}

/// Every open and merged pull request whose head is `branch`.
pub(crate) fn branch_pull_requests(
    git_dir: &Utf8Path,
    branch: &str,
) -> Result<BranchPullRequests, Failure> {
    let mut observed = BranchPullRequests::default();
    for pr in list_pull_requests(git_dir, branch, "all")? {
        if pr.state == OPEN {
            observed.open.get_or_insert(pr);
        } else if pr.is_merged() {
            observed.merged.push(pr);
        }
    }
    observed
        .merged
        .sort_by_key(|pr| std::cmp::Reverse(pr.number));
    Ok(observed)
}
