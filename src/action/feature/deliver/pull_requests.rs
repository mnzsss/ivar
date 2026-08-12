//! The pull-request half of delivery: detecting an existing PR, creating one
//! for a feature branch, and linking sibling PRs once every URL is known.

use camino::Utf8Path;
use serde::Deserialize;

use crate::domain::name::{BranchName, FeatureName};
use crate::error::{Failure, FixAction};
use crate::infra::proc;

/// The URL of the open pull request for `branch`, when there is one.
///
/// Runs `gh pr list --head <branch> --state open --json url` and returns the
/// first entry's URL. A non-zero exit or spawn failure means no PR — the branch
/// simply hasn't been promoted through GitHub yet.
///
/// The URL is the answer, not a by-product: delivering a branch that already
/// has a PR must report *that* PR, and `gh pr create` refuses to hand it over
/// (it exits non-zero on a duplicate), so this is the only place it comes from.
pub(crate) fn existing_pr_url(git_dir: &Utf8Path, branch: &str) -> Option<String> {
    let output = proc::capture(
        &proc::Command::new("gh")
            .args([
                "pr", "list", "--head", branch, "--state", "open", "--json", "url",
            ])
            .cwd(git_dir),
    );

    let output = match output {
        Ok(o) if o.success() => o.stdout,
        _ => return None,
    };

    #[derive(Debug, Clone, Deserialize)]
    struct GhListEntry {
        url: String,
    }

    let entries: Vec<GhListEntry> = serde_json::from_str(&output).unwrap_or_default();
    entries.into_iter().next().map(|entry| entry.url)
}

/// Create a pull request for the feature branch via `gh pr create`.
///
/// Returns the PR URL when successful. The base is the repo's default branch
/// and the head is the feature branch. The body carries the feature name so
/// the PR is traceable back to its parent.
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

    // `gh pr create` has no `--json` flag — that is `pr list` and `pr view` —
    // and passing one fails the whole invocation on an unknown flag. The URL is
    // read off stdout instead, where gh prints it.
    pr_url(&output.stdout).ok_or_else(|| {
        Failure::failed(
            "deliver.pr_parse_failed",
            format!("could not read the PR URL `gh` printed for `{branch}`"),
        )
        .expected("the URL of the created pull request on stdout")
        .actual(output.diagnostic())
    })
}

/// The pull-request URL in `stdout`: the last line that is one.
fn pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://") && line.contains("/pull/"))
        .map(ToOwned::to_owned)
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
