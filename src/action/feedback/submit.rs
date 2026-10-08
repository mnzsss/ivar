use super::super::discover_hall;
use super::{DEFAULT_REPO, URL_LIMIT, issue_preview, redactions};
use crate::action::Ctx;
use crate::domain::feedback::FeedbackStatus;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::url::encode_component;
use crate::store::feedback as feedback_store;
use serde::{Deserialize, Serialize};
use std::io;

pub trait GhClient {
    fn gh_login(&self) -> Result<String, Failure>;
    fn gh_issue_create(&self, repo: &str, title: &str, body: &str) -> Result<String, Failure>;
}
#[derive(Debug)]
pub struct LiveGh;

impl GhClient for LiveGh {
    fn gh_login(&self) -> Result<String, Failure> {
        crate::infra::github::gh_login()
    }

    fn gh_issue_create(&self, repo: &str, title: &str, body: &str) -> Result<String, Failure> {
        crate::infra::github::gh_stdout_with_stdin(
            &[
                "issue",
                "create",
                "--repo",
                repo,
                "--title",
                title,
                "--body-file",
                "-",
            ],
            body,
            "feedback.gh_create_failed",
        )
    }
}

/// How `submit` gets a human's approval (R-FB-HUMAN).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum SubmitMode {
    /// Ask on the terminal, then publish.
    Interactive,
    /// Show exactly what would be published, and its fingerprint. Publishes nothing.
    Preview,
    /// Publish only if the entry still fingerprints as the approved preview.
    Apply { fingerprint: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubmitInput {
    pub id: String,
    pub repo: Option<String>,
    pub mode: SubmitMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum SubmitReport {
    Published {
        id: String,
        url: String,
    },
    Prefilled {
        id: String,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },
    Declined {
        id: String,
    },
    Preview {
        id: String,
        repo: String,
        title: String,
        body: String,
        fingerprint: String,
    },
}

impl WriteHuman for SubmitReport {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        match self {
            Self::Published { id, url } => {
                writeln!(w, "Published feedback {id}: {url}")
            }
            Self::Prefilled { id, url, body } => {
                writeln!(
                    w,
                    "gh is not authenticated. Created prefilled GitHub issue URL for {id}:"
                )?;
                writeln!(w, "{url}")?;
                if let Some(b) = body {
                    writeln!(
                        w,
                        "\nBody exceeds URL limit. Please paste the following content:\n\n{b}"
                    )?;
                }
                Ok(())
            }
            Self::Declined { id } => {
                writeln!(w, "Submission declined for {id}.")
            }
            Self::Preview {
                id,
                repo,
                title,
                body,
                fingerprint,
            } => {
                writeln!(w, "Preview of feedback {id} for {repo}")?;
                writeln!(w, "\nTitle: {title}\n\n{body}\n")?;
                writeln!(w, "Fingerprint: {fingerprint}")?;
                writeln!(
                    w,
                    "Publish after human approval: ivar feedback submit {id} --repo {repo} --fingerprint {fingerprint}"
                )
            }
        }
    }
}

/// SHA-256 of the canonical JSON of what gets published and where: the
/// entry id, the target repo, and the redacted title and body.
pub(crate) fn fingerprint(
    id: &str,
    repo: &str,
    title: &str,
    body: &str,
) -> Result<String, Failure> {
    #[derive(Serialize)]
    struct Published<'a> {
        id: &'a str,
        repo: &'a str,
        title: &'a str,
        body: &'a str,
    }
    let rendered = crate::infra::json::to_canonical_string(&Published {
        id,
        repo,
        title,
        body,
    })?;
    Ok(crate::infra::hash::text(&rendered))
}

fn fingerprint_mismatch(id: &str, repo: &str, expected: &str, actual: &str) -> Failure {
    let command = format!("ivar feedback submit {id} --repo {repo} --preview");
    Failure::blocked(
        "feedback.fingerprint_mismatch",
        format!("feedback {id} no longer matches the preview that was approved"),
    )
    .expected(format!("the preview fingerprint `{expected}`"))
    .actual(format!(
        "the entry, its redaction or the repo now fingerprint as `{actual}`"
    ))
    .fix(
        FixAction::safe(
            "feedback.re_preview",
            format!("Preview again with `{command}` and have a human approve the new text."),
        )
        .command(command),
    )
}

pub fn submit(ctx: &Ctx, input: SubmitInput) -> Outcome<SubmitReport> {
    submit_with(ctx, input, &LiveGh)
}

pub fn submit_with<G: GhClient>(ctx: &Ctx, input: SubmitInput, gh: &G) -> Outcome<SubmitReport> {
    let SubmitInput { id, repo, mode } = input;
    let layout = discover_hall(ctx)?;

    let mut entry = feedback_store::read(&layout, &id)?.ok_or_else(|| {
        Failure::blocked(
            "feedback.not_found",
            format!("Feedback entry '{id}' not found"),
        )
    })?;
    if entry.frontmatter.status == FeedbackStatus::Published {
        let url_str = entry
            .frontmatter
            .published_url
            .as_deref()
            .unwrap_or("<unknown URL>");
        return Err(Failure::blocked(
            "feedback.already_published",
            format!("Feedback entry '{id}' is already published at {url_str}"),
        ));
    }

    if !entry.is_writable() {
        return Err(Failure::blocked(
            "feedback.unreadable",
            format!("Feedback entry '{id}' has unknown or unparseable frontmatter"),
        ));
    }

    let r = redactions(&layout);
    let (redacted_title, redacted_body) = issue_preview(&entry, &r);
    let repo = repo.unwrap_or_else(|| DEFAULT_REPO.to_owned());
    let current = fingerprint(&entry.id, &repo, &redacted_title, &redacted_body)?;

    match mode {
        SubmitMode::Preview => {
            return Ok(Report::new(SubmitReport::Preview {
                id: entry.id,
                repo,
                title: redacted_title,
                body: redacted_body,
                fingerprint: current,
            }));
        }
        SubmitMode::Apply {
            fingerprint: expected,
        } => {
            if expected != current {
                return Err(fingerprint_mismatch(&entry.id, &repo, &expected, &current));
            }
        }
        SubmitMode::Interactive => {
            // R-FB-HUMAN: Gate on confirmation seam interactivity
            if !ctx.confirm.is_interactive() {
                return Err(Failure::blocked(
                    "feedback.submit_needs_terminal",
                    "Feedback submission requires an interactive terminal for confirmation",
                ));
            }
            let question = format!(
                "Submit feedback to GitHub?\n\nTitle: {}\n\n{}",
                redacted_title, redacted_body
            );
            if !ctx.confirm(&question, None)? {
                return Ok(Report::new(SubmitReport::Declined { id: entry.id }));
            }
        }
    }
    let repo = repo.as_str();

    // Try gh issue create
    if gh.gh_login().is_ok()
        && let Ok(url) = gh.gh_issue_create(repo, &redacted_title, &redacted_body)
    {
        let trimmed_url = url.trim().to_owned();
        entry.frontmatter.status = FeedbackStatus::Published;
        entry.frontmatter.published_url = Some(trimmed_url.clone());
        feedback_store::write(&layout, &entry)?;
        return Ok(Report::new(SubmitReport::Published {
            id: entry.id,
            url: trimmed_url,
        }));
    }

    // Fallback URL (R-FB-URL)
    let enc_title = encode_component(&redacted_title, false);
    let enc_body = encode_component(&redacted_body, false);
    let full_url =
        format!("https://github.com/{repo}/issues/new?title={enc_title}&body={enc_body}");

    if full_url.len() <= URL_LIMIT {
        Ok(Report::new(SubmitReport::Prefilled {
            id: entry.id,
            url: full_url,
            body: None,
        }))
    } else {
        let title_only_url = format!("https://github.com/{repo}/issues/new?title={enc_title}");
        Ok(Report::new(SubmitReport::Prefilled {
            id: entry.id,
            url: title_only_url,
            body: Some(redacted_body),
        }))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feedback/submit.rs"]
mod tests;
