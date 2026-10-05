use std::time::{Duration, Instant};

use camino::Utf8Path;
use serde::Deserialize;

use super::{GhPrRecord, PullRequest, capture, parse_gh};
use crate::domain::feature::{IntegrationStrategy, PrCheckResult};
use crate::error::{Failure, FixAction};
use crate::infra::proc;

/// How often [`observe_merge`] polls `gh pr view`.
const OBSERVE_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long [`observe_merge`] waits for a merge before reporting it pending.
const OBSERVE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
    let checks: Vec<GhCheck> = parse_gh(&output.stdout, "pr checks")?;
    Ok(checks
        .into_iter()
        .map(|check| PrCheckResult {
            name: check.name,
            bucket: check.bucket,
        })
        .collect())
}

/// Explicitly request the merge of the PR at `url`, mapping `strategy` to the
/// one matching flag. `subject` is the merge commit's subject for `squash`
/// and `merge`; `rebase` creates no merge commit and passes none. Always
/// passes `--match-head-commit <source_sha>`, never `--admin`, and never
/// deletes the branch — protection, auto-merge, and merge queues are `gh`'s
/// business and it keeps them.
pub(crate) fn request_merge(
    git_dir: &Utf8Path,
    url: &str,
    source_sha: &str,
    strategy: IntegrationStrategy,
    subject: &str,
) -> Result<(), Failure> {
    let flag = match strategy {
        IntegrationStrategy::Merge => "--merge",
        IntegrationStrategy::Squash => "--squash",
        IntegrationStrategy::Rebase => "--rebase",
    };
    let mut args = vec!["pr", "merge", url, flag, "--match-head-commit", source_sha];
    if strategy != IntegrationStrategy::Rebase {
        args.extend(["--subject", subject]);
    }
    capture(
        &proc::Command::new("gh").args(args).cwd(git_dir),
        "pr merge",
    )?;
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
        if pr.is_merged() {
            return Ok(pr);
        }
        match pr.state.as_str() {
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

/// The forge state of the pull request at `url`: `OPEN`, `MERGED`, `CLOSED`.
pub(crate) fn pull_request_state(git_dir: &Utf8Path, url: &str) -> Result<String, Failure> {
    view_pull_request(git_dir, url).map(|pr| pr.state)
}

/// One `gh pr view` — the observation primitive.
fn view_pull_request(git_dir: &Utf8Path, url: &str) -> Result<PullRequest, Failure> {
    let output = capture(
        &proc::Command::new("gh")
            .args([
                "pr",
                "view",
                url,
                "--json",
                "url,number,state,mergeCommit,headRefOid,isDraft",
            ])
            .cwd(git_dir),
        "pr view",
    )?;
    let record: GhPrRecord = parse_gh(&output, "pr view")?;
    Ok(record.into())
}
