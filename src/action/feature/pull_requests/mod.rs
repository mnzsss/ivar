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

mod lookup;
mod mutate;
mod observation;
mod siblings;

pub(crate) use lookup::{existing_pr, find_pull_request, list_pull_requests};
pub(crate) use mutate::{convert_pull_request_to_draft, create_pull_request, edit_pull_request};
pub(crate) use observation::{observe_merge, pull_request_state, request_merge, required_checks};
pub(crate) use siblings::link_sibling_prs;

use serde::Deserialize;

use crate::error::{Failure, FixAction};
use crate::infra::proc;

/// The forge's state word for a pull request that has landed.
pub(crate) const MERGED: &str = "MERGED";

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
    /// Is this PR a draft?
    pub is_draft: bool,
}

impl PullRequest {
    pub(crate) fn is_merged(&self) -> bool {
        self.state == MERGED
    }

    pub(crate) fn merged_head(&self, head: &str) -> bool {
        self.is_merged() && self.head_oid.as_deref() == Some(head)
    }
}

/// The `--json url,number,state,mergeCommit,headRefOid,isDraft` shape `gh pr
/// list` and `gh pr view` both emit.
#[derive(Debug, Deserialize)]
pub(super) struct GhPrRecord {
    pub(super) url: String,
    #[serde(default)]
    pub(super) number: u64,
    #[serde(default)]
    pub(super) state: String,
    #[serde(default, rename = "isDraft")]
    pub(super) is_draft: bool,
    #[serde(default, rename = "mergeCommit")]
    pub(super) merge_commit: Option<GhOid>,
    #[serde(default, rename = "headRefOid")]
    pub(super) head_ref_oid: Option<String>,
}

/// `mergeCommit` is an object `{"oid": …}` when merged, `null` otherwise.
#[derive(Debug, Deserialize)]
pub(super) struct GhOid {
    pub(super) oid: String,
}

impl From<GhPrRecord> for PullRequest {
    fn from(record: GhPrRecord) -> Self {
        Self {
            url: record.url,
            number: record.number,
            state: record.state,
            is_draft: record.is_draft,
            head_oid: record.head_ref_oid,
            merge_commit: record.merge_commit.map(|commit| commit.oid),
        }
    }
}

/// Run a `gh` command, turning a non-zero exit (or spawn failure) into a
/// strict [`Failure`] naming the operation and carrying git/gh's own
/// diagnostic.
pub(super) fn capture(command: &proc::Command, operation: &str) -> Result<String, Failure> {
    let output = proc::capture(command)?;
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

pub(super) fn parse_gh<T: serde::de::DeserializeOwned>(
    output: &str,
    operation: &str,
) -> Result<T, Failure> {
    serde_json::from_str(output).map_err(|source| {
        Failure::failed(
            "pull_requests.parse_failed",
            format!("could not parse `gh {operation}` output: {source}"),
        )
        .actual(output.to_owned())
    })
}

/// `gh` may hand back CRLF line endings and surrounding whitespace that the
/// text ivar sends never had; neither is a real difference.
pub(super) fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_owned()
}

/// The pull-request URL in `stdout`: the last line that is one.
pub(super) fn pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://") && line.contains("/pull/"))
        .map(ToOwned::to_owned)
}

/// The PR number trailing a `.../pull/<number>` URL. `gh pr create` prints no
/// `--json`, so the number is parsed off the same URL its stdout gives up.
pub(super) fn pr_number(url: &str) -> Option<u64> {
    url.rsplit('/').next()?.parse().ok()
}
