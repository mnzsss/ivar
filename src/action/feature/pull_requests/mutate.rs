use camino::Utf8Path;
use serde::Deserialize;

use super::{PullRequest, capture, normalized, parse_gh, pr_number, pr_url};
use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction};
use crate::infra::proc;

/// Create a pull request from `head` into `base` for `feature`. Returns the
/// created PR. The title and body carry the feature name so the PR is
/// traceable back to its parent. When `title` is `None`, the historical default
/// ("Part of feature `{feature}`.") is used; when `body` is `None`, the same
/// default body is used. Callers pass `repo.pr_title`/`repo.pr_body` which may
/// be `None` to preserve defaults.
pub(crate) fn create_pull_request(
    git_dir: &Utf8Path,
    head: &BranchName,
    base: &BranchName,
    feature: &FeatureName,
    title: Option<&str>,
    body: Option<&str>,
    draft: bool,
) -> Result<PullRequest, Failure> {
    let default_title = feature.to_string();
    let default_body = format!("Part of feature `{feature}`.");
    let effective_title = title.unwrap_or(default_title.as_str());
    let effective_body = body.unwrap_or(default_body.as_str());

    let mut args = vec![
        "pr",
        "create",
        "--base",
        base.as_str(),
        "--head",
        head.as_str(),
        "--title",
        effective_title,
        "--body",
        effective_body,
    ];
    if draft {
        args.push("--draft");
    }

    let output = capture(
        &proc::Command::new("gh").args(args).cwd(git_dir),
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
        is_draft: draft,
        head_oid: None,
        merge_commit: None,
    })
}

/// Convert an existing pull request to draft.
pub(crate) fn convert_pull_request_to_draft(git_dir: &Utf8Path, url: &str) -> Result<(), Failure> {
    let _ = capture(
        &proc::Command::new("gh")
            .args(["pr", "ready", "--undo", url])
            .cwd(git_dir),
        "pr ready --undo",
    )?;
    Ok(())
}

/// Edit a pull request at `url` with optional `title` and `body`.
/// Only non-None fields are forwarded to `gh pr edit`; absent fields
/// are left unchanged, and fields already matching the PR are skipped, so a
/// re-delivery with the same metadata makes no edit.
pub(crate) fn edit_pull_request(
    git_dir: &Utf8Path,
    url: &str,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<(), Failure> {
    // When both title and body are absent, this is a no-op — no `gh` invocation.
    if title.is_none() && body.is_none() {
        return Ok(());
    }

    // Reading the current metadata only avoids redundant edits; when it
    // fails, every requested field is sent as before.
    let (title, body) = match view_metadata(git_dir, url) {
        Ok(current) => (
            title.filter(|t| normalized(t) != normalized(&current.title)),
            body.filter(|b| normalized(b) != normalized(&current.body)),
        ),
        Err(_) => (title, body),
    };
    if title.is_none() && body.is_none() {
        return Ok(());
    }

    // `gh pr edit` takes the PR as a positional argument -- `[<number> | <url>
    // | <branch>]`. There is no `--url` flag; passing one aborts with
    // `unknown flag: --url` before any edit is attempted.
    let mut args = vec!["pr", "edit", url];
    if let Some(t) = title {
        args.push("--title");
        args.push(t);
    }
    if let Some(b) = body {
        args.push("--body");
        args.push(b);
    }

    let cmd = proc::Command::new("gh").args(args).cwd(git_dir);

    let output = proc::capture(&cmd)?;
    // gh pr edit exits non-zero only on real errors (not "no change");
    // treat a non-zero exit with no diagnostic as a no-op success,
    // and propagate structured failures.
    if !output.success() {
        return Err(Failure::failed(
            "deliver.pr_edit_failed",
            format!("`gh pr edit` failed: {}", output.diagnostic()),
        )
        .expected("gh pr edit to succeed or be a no-op")
        .actual(output.diagnostic())
        .fix(FixAction::safe(
            "deliver.pr_edit_retry",
            "Run `gh pr edit <url>` with corrected flags.",
        )));
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct PrMetadata {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
}

fn view_metadata(git_dir: &Utf8Path, url: &str) -> Result<PrMetadata, Failure> {
    let output = capture(
        &proc::Command::new("gh")
            .args(["pr", "view", url, "--json", "title,body"])
            .cwd(git_dir),
        "pr view",
    )?;
    parse_gh(&output, "pr view")
}
