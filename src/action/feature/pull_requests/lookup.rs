use camino::Utf8Path;

use super::{GhPrRecord, PullRequest, capture, parse_gh};
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
                "url,number,state,mergeCommit,headRefOid,isDraft",
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
