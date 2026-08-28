//! The shared pull-request operations: finding, creating, checking, merging,
//! and observing PRs through the `gh` executable.
//!
//! Owned by the feature module because two features use it: delivery opens PRs
//! for root features, and nested integration observes and merges a child's PR
//! into its immediate parent. One command-construction site, one contract.
//!
//! The public vocabulary is `via=pr|local` — `github` is never a via. That the
//! PR implementation happens to use `gh` is an implementation detail of
//! `via=pr`, owned here. [`find_pull_request`] and the observation loop are
//! strict: an apply path that cannot answer refuses, never guesses.

use std::time::{Duration, Instant};

use camino::Utf8Path;
use serde::Deserialize;

use crate::domain::feature::{IntegrationStrategy, PrCheckResult};
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction};
use crate::infra::proc;

/// How often [`observe_merge`] polls `gh pr view`.
const OBSERVE_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long [`observe_merge`] waits for a merge before reporting it pending.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A pull request as `gh` reports it — the fields ivar reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequest {
    /// The PR's URL.
    pub url: String,
    /// The host-assigned PR number — what a blocker names alongside the URL
    /// so a human can find it without following a link.
    pub number: u64,
    /// The PR's state: `OPEN`, `MERGED`, `CLOSED`, `QUEUED`, …
    pub state: String,
    /// The head branch's commit, per the forge. `None` when the record does
    /// not carry it.
    pub head_oid: Option<String>,
    /// The merge commit, once merged. `None` while open.
    pub merge_commit: Option<String>,
}

/// The `--json url,number,state,mergeCommit,headRefOid` shape `gh pr list`
/// and `gh pr view` both emit.
#[derive(Debug, Deserialize)]
struct GhPrRecord {
    url: String,
    #[serde(default)]
    number: u64,
    #[serde(default)]
    state: String,
    #[serde(default, rename = "mergeCommit")]
    merge_commit: Option<GhOid>,
    #[serde(default, rename = "headRefOid")]
    head_ref_oid: Option<String>,
}

/// `mergeCommit` is an object `{"oid": …}` when merged, `null` otherwise.
#[derive(Debug, Deserialize)]
struct GhOid {
    oid: String,
}

impl From<GhPrRecord> for PullRequest {
    fn from(record: GhPrRecord) -> Self {
        Self {
            url: record.url,
            number: record.number,
            state: record.state,
            head_oid: record.head_ref_oid,
            merge_commit: record.merge_commit.map(|commit| commit.oid),
        }
    }
}

/// Find the pull request whose head is `branch` and whose state is `state`
/// (`open` or `all`). `Ok(None)` when there is none; a `gh` failure is a
/// strict error, never a silent "no PR".
pub(crate) fn find_pull_request(
    git_dir: &Utf8Path,
    head: &str,
    state: &str,
) -> Result<Option<PullRequest>, Failure> {
    let output = capture(
        proc::Command::new("gh")
            .args([
                "pr",
                "list",
                "--head",
                head,
                "--state",
                state,
                "--json",
                "url,number,state,mergeCommit,headRefOid",
            ])
            .cwd(git_dir),
        "pr list",
    )?;
    let records: Vec<GhPrRecord> = serde_json::from_str(&output).map_err(|source| {
        Failure::failed(
            "pull_requests.parse_failed",
            format!("could not parse `gh pr list` output: {source}"),
        )
        .actual(output.clone())
    })?;
    Ok(records.into_iter().next().map(PullRequest::from))
}

/// Create a pull request from `head` into `base` for `feature`. Returns the
/// created PR. The title and body carry the feature name so the PR is
/// traceable back to its parent.
pub(crate) fn create_pull_request(
    git_dir: &Utf8Path,
    head: &BranchName,
    base: &BranchName,
    feature: &FeatureName,
) -> Result<PullRequest, Failure> {
    let output = capture(
        proc::Command::new("gh")
            .args([
                "pr",
                "create",
                "--base",
                base.as_str(),
                "--head",
                head.as_str(),
                "--title",
                &feature.to_string(),
                "--body",
                &format!("Part of feature `{feature}`."),
            ])
            .cwd(git_dir),
        "pr create",
    )?;

    // `gh pr create` has no `--json` flag — that is `pr list` and `pr view` —
    // and passing one fails the whole invocation on an unknown flag. The URL
    // is read off stdout instead, where gh prints it.
    let url = pr_url(&output).ok_or_else(|| {
        Failure::failed(
            "deliver.pr_parse_failed",
            format!("could not read the PR URL `gh` printed for `{head}`"),
        )
        .actual(output)
    })?;
    let number = pr_number(&url).unwrap_or(0);
    Ok(PullRequest {
        url,
        number,
        state: "OPEN".to_owned(),
        head_oid: None,
        merge_commit: None,
    })
}

/// The required checks on the PR at `url`, as the forge reported them.
///
/// Pending is data, not an error — the caller treats it as a resumable
/// blocked result. A hard `gh` failure is a strict error.
pub(crate) fn required_checks(
    git_dir: &Utf8Path,
    url: &str,
) -> Result<Vec<PrCheckResult>, Failure> {
    let output = proc::capture(
        &proc::Command::new("gh")
            .args([
                "pr",
                "checks",
                url,
                "--required",
                "--json",
                "name,bucket,state,link",
            ])
            .cwd(git_dir),
    )?;
    // The real `gh pr checks` exits 8 while anything is pending; the output
    // is still the answer. Anything else non-zero is a hard refusal.
    if !output.success() && output.code != Some(8) {
        return Err(Failure::failed(
            "pull_requests.checks_failed",
            format!("`gh pr checks` could not report on {url}"),
        )
        .expected("gh to be authenticated and the PR to exist")
        .actual(output.diagnostic())
        .fix(FixAction::safe(
            "pull_requests.check_auth",
            "Ensure `gh auth status` is OK and the branch was pushed.",
        )));
    }

    #[derive(Debug, Deserialize)]
    struct GhCheck {
        name: String,
        #[serde(default)]
        bucket: String,
    }
    let checks: Vec<GhCheck> = serde_json::from_str(&output.stdout).map_err(|source| {
        Failure::failed(
            "pull_requests.parse_failed",
            format!("could not parse `gh pr checks` output: {source}"),
        )
        .actual(output.stdout.clone())
    })?;
    Ok(checks
        .into_iter()
        .map(|check| PrCheckResult {
            name: check.name,
            bucket: check.bucket,
        })
        .collect())
}

/// Explicitly request the merge of the PR at `url`, mapping `strategy` to the
/// one matching flag. Always passes `--match-head-commit <source_sha>`, never
/// `--admin`, and never deletes the branch — protection, auto-merge, and
/// merge queues are `gh`'s business and it keeps them.
pub(crate) fn request_merge(
    git_dir: &Utf8Path,
    url: &str,
    source_sha: &str,
    strategy: IntegrationStrategy,
) -> Result<(), Failure> {
    let flag = match strategy {
        IntegrationStrategy::Merge => "--merge",
        IntegrationStrategy::Squash => "--squash",
        IntegrationStrategy::Rebase => "--rebase",
    };
    let output = capture(
        proc::Command::new("gh")
            .args(["pr", "merge", url, flag, "--match-head-commit", source_sha])
            .cwd(git_dir),
        "pr merge",
    )?;
    let _ = output;
    Ok(())
}
/// Observe the PR at `url` until it merges: poll `gh pr view` every
/// [`OBSERVE_POLL_INTERVAL`] for at most [`OBSERVE_TIMEOUT`]. `MERGED`
/// returns the PR (whose `merge_commit` is the result); `CLOSED` fails;
/// a timeout reports the merge as pending and resumable.
pub(crate) fn observe_merge(git_dir: &Utf8Path, url: &str) -> Result<PullRequest, Failure> {
    observe_merge_with(git_dir, url, OBSERVE_POLL_INTERVAL, OBSERVE_TIMEOUT)
}

/// The same observation loop with injected durations, so tests can drive it
/// without sleeping.
fn observe_merge_with(
    git_dir: &Utf8Path,
    url: &str,
    poll: Duration,
    timeout: Duration,
) -> Result<PullRequest, Failure> {
    let deadline = Instant::now() + timeout;
    loop {
        let pr = view_pull_request(git_dir, url)?;
        match pr.state.as_str() {
            "MERGED" => return Ok(pr),
            "CLOSED" => {
                return Err(Failure::failed(
                    "integration.pr_closed",
                    format!("the pull request {url} was closed without merging"),
                )
                .expected("the PR to merge")
                .actual("the PR is closed")
                .fix(FixAction::safe(
                    "integration.reopen_or_recreate",
                    "Reopen the PR, or create a fresh child and re-integrate.",
                )));
            }
            _ => {
                if Instant::now() >= deadline {
                    return Err(Failure::blocked(
                        "integration.pr_pending",
                        format!("the pull request {url} has not merged yet"),
                    )
                    .expected("the PR to merge within the observation window")
                    .actual("the PR is still open or queued")
                    .fix(FixAction::safe(
                        "integration.observe_again",
                        "Run `ivar feature integrate` again to re-observe the merge.",
                    )));
                }
                std::thread::sleep(poll);
            }
        }
    }
}

/// One `gh pr view` — the observation primitive.
fn view_pull_request(git_dir: &Utf8Path, url: &str) -> Result<PullRequest, Failure> {
    let output = capture(
        proc::Command::new("gh")
            .args([
                "pr",
                "view",
                url,
                "--json",
                "url,number,state,mergeCommit,headRefOid",
            ])
            .cwd(git_dir),
        "pr view",
    )?;
    let record: GhPrRecord = serde_json::from_str(&output).map_err(|source| {
        Failure::failed(
            "pull_requests.parse_failed",
            format!("could not parse `gh pr view` output: {source}"),
        )
        .actual(output.clone())
    })?;
    Ok(record.into())
}

// -- delivery compatibility -------------------------------------------------
//
// Delivery's preview may collapse lookup errors to "new PR" where it
// intentionally does; apply uses the strict [`find_pull_request`]. The two
// helpers below keep the preview's best-effort shape.

/// The URL of the open pull request for `branch`, when there is one —
/// best-effort: a `gh` failure means "no PR", which is the preview's
/// intentional answer.
pub(crate) fn existing_pr_url(git_dir: &Utf8Path, branch: &str) -> Option<String> {
    find_pull_request(git_dir, branch, "open")
        .ok()
        .flatten()
        .map(|pr| pr.url)
}

/// Add a comment to each PR linking it to its siblings.
///
/// Every sibling PR gets a comment noting the other PRs in the batch — always
/// with "part of" language, never "depends on". The comment uses the header
/// `## Sibling PRs:` so repeated runs do not duplicate content.
pub(crate) fn link_sibling_prs(pr_urls: &[String]) {
    for (i, url) in pr_urls.iter().enumerate() {
        let others: Vec<&str> = pr_urls
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, u)| u.as_str())
            .collect();

        if others.is_empty() {
            continue;
        }

        let mut body =
            String::from("## Sibling PRs:\n\nThis PR is part of feature delivery alongside:\n\n");
        for other in &others {
            body.push_str("- ");
            body.push_str(other);
            body.push('\n');
        }

        let _ =
            proc::capture(&proc::Command::new("gh").args(["pr", "comment", url, "--body", &body]));
    }
}

/// Run a `gh` command, turning a non-zero exit (or spawn failure) into a
/// strict [`Failure`] naming the operation and carrying git/gh's own
/// diagnostic.
fn capture(command: proc::Command, operation: &str) -> Result<String, Failure> {
    let output = proc::capture(&command)?;
    if output.success() {
        return Ok(output.stdout);
    }
    Err(Failure::failed(
        "pull_requests.command_failed",
        format!("`gh {operation}` failed: {}", output.diagnostic()),
    )
    .expected("gh to be installed, authenticated, and the repository reachable")
    .actual(output.diagnostic())
    .fix(FixAction::safe(
        "pull_requests.check_gh",
        "Ensure `gh` is installed and `gh auth status` is OK.",
    )))
}

/// The pull-request URL in `stdout`: the last line that is one.
fn pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://") && line.contains("/pull/"))
        .map(ToOwned::to_owned)
}

/// The PR number trailing a `.../pull/<number>` URL. `gh pr create` prints no
/// `--json`, so the number is parsed off the same URL its stdout gives up.
fn pr_number(url: &str) -> Option<u64> {
    url.rsplit('/').next()?.parse().ok()
}
