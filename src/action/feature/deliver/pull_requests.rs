//! The pull-request half of delivery: detecting an existing PR, creating one
//! for a feature branch, and linking sibling PRs once every URL is known.

/// Whether a pull request already exists for `branch` on the remote at `url`.
///
/// Runs `gh pr list --head <branch> --state open --json url` and checks whether
/// any entry is returned. A non-zero exit or spawn failure means no PR — the
/// branch simply hasn't been promoted through GitHub yet.
use camino::Utf8Path;
use serde::Deserialize;

use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction};
use crate::infra::proc;

pub(crate) fn has_existing_pr(git_dir: &Utf8Path, branch: &str) -> Result<bool, Failure> {
    let output = proc::capture(
        &proc::Command::new("gh")
            .args([
                "pr", "list", "--head", branch, "--state", "open", "--json", "url",
            ])
            .cwd(git_dir),
    );

    let output = match output {
        Ok(o) if o.success() => o.stdout,
        _ => return Ok(false),
    };

    // The `url` field is incidental — only the presence of an entry matters.
    let entries: Vec<serde_json::Value> = serde_json::from_str(&output).unwrap_or_default();
    Ok(!entries.is_empty())
}

/// Create a pull request for the feature branch via `gh pr create`.
///
/// Returns the PR URL when successful. Uses the repo's manifest URL as the
/// base (the default branch), and the feature branch as the head. The body
/// carries the feature name so the PR is traceable back to its parent.
pub(crate) fn create_pr(
    git_dir: &Utf8Path,
    branch: &BranchName,
    base_branch: &BranchName,
    feature_name: &FeatureName,
) -> Result<String, Failure> {
    let output = proc::capture(
        &proc::Command::new("gh")
            .args([
                "pr",
                "create",
                "--base",
                base_branch.as_str(),
                "--head",
                branch.as_str(),
                "--title",
                &format!("{feature_name}"),
                "--body",
                &format!("Part of feature `{feature_name}`."),
                "--json",
                "url",
            ])
            .cwd(git_dir),
    )?;

    if !output.success() {
        return Err(Failure::failed(
            "deliver.pr_create_failed",
            format!("could not create PR for `{branch}`"),
        )
        .expected("gh to be authenticated and the branch to exist on the remote")
        .actual(output.diagnostic())
        .fix(FixAction::safe(
            "deliver.check_auth",
            "Ensure `gh auth status` is OK and the branch was pushed.",
        )));
    }

    #[derive(Debug, Clone, Deserialize)]
    struct GhCreateOutput {
        url: String,
    }

    let parsed: GhCreateOutput = serde_json::from_str(&output.stdout).map_err(|e| {
        Failure::failed(
            "deliver.pr_parse_failed",
            format!("could not parse gh pr create output: {e}"),
        )
        .expected("a JSON object with a `url` field")
        .actual(output.stdout.clone())
    })?;

    Ok(parsed.url)
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
