use serde::Deserialize;

use super::{capture, normalized, parse_gh};
use crate::error::Failure;
use crate::infra::proc;

const SIBLING_HEADER: &str = "## Sibling PRs:";

/// Add a comment to each PR linking it to its siblings.
///
/// Every sibling PR gets a comment noting the other PRs in the batch — always
/// with "part of" language, never "depends on". The comment is found by its
/// `## Sibling PRs:` header: it is created when missing, edited in place when
/// its sibling list changed, and left alone otherwise.
pub(crate) fn link_sibling_prs(pr_urls: &[String]) {
    let login = capture(
        &proc::Command::new("gh").args(["api", "user", "--jq", ".login"]),
        "api user",
    )
    .ok()
    .map(|login| login.trim().to_owned());
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
            format!("{SIBLING_HEADER}\n\nThis PR is part of feature delivery alongside:\n\n");
        for other in &others {
            body.push_str("- ");
            body.push_str(other);
            body.push('\n');
        }

        // Posting without knowing the existing comments would duplicate ours.
        let Ok(existing) = sibling_comments(url) else {
            continue;
        };
        match existing.into_iter().find(|comment| {
            // Unknown login: match on header alone, risking adopting a foreign
            // header comment rather than duplicating ours on every delivery.
            comment.body.starts_with(SIBLING_HEADER)
                && login
                    .as_deref()
                    .is_none_or(|login| comment.author.login == login)
        }) {
            None => {
                let _ = proc::capture(
                    &proc::Command::new("gh").args(["pr", "comment", url, "--body", &body]),
                );
            }
            Some(comment) if normalized(&comment.body) != normalized(&body) => {
                update_comment(&comment.id, &body);
            }
            Some(_) => {}
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhComment {
    id: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    author: GhAuthor,
}

#[derive(Debug, Default, Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhComments {
    #[serde(default)]
    comments: Vec<GhComment>,
}

fn sibling_comments(url: &str) -> Result<Vec<GhComment>, Failure> {
    let output = capture(
        &proc::Command::new("gh").args(["pr", "view", url, "--json", "comments"]),
        "pr view",
    )?;
    parse_gh::<GhComments>(&output, "pr view").map(|parsed| parsed.comments)
}

fn update_comment(id: &str, body: &str) {
    let _ = proc::capture(&proc::Command::new("gh").args([
        "api",
        "graphql",
        "-f",
        "query=mutation($id: ID!, $body: String!) { updateIssueComment(input: {id: $id, body: $body}) { clientMutationId } }",
        "-f",
        &format!("id={id}"),
        "-f",
        &format!("body={body}"),
    ]));
}
