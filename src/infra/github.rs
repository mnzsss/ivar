//! GitHub authentication and API helpers.
//!
//! # Auth cascade
//!
//! Tries sources in preference order:
//!
//! 1. `gh auth token` — the GitHub CLI's own token (preferred, respects SSO).
//! 2. `GITHUB_TOKEN` environment variable.
//! 3. `GH_TOKEN` environment variable.
//!
//! If none succeed, returns [`Failure::blocked`] naming the output that was
//! expected — never degrades silently to anonymous access.
//!
//! # Module boundaries
//!
//! `infra` may import [`crate::error`] and nothing else from this crate.
//! This module reaches into `std::process` and `ureq` — the network boundary.

use std::io::Read;
use std::process::Command;

use crate::error::{Failure, FixAction};

/// Everything that can go wrong getting a GitHub token.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `gh auth token` ran but exited non-zero or returned empty output.
    #[error("`gh auth token` exited {code}: {stderr}")]
    GhFailed { code: i32, stderr: String },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::GhFailed { code, stderr } => {
                Failure::failed("github.gh_failed", format!("`gh auth token` exited {code}"))
                    .expected("gh to be authenticated")
                    .actual(stderr)
                    .fix(
                        FixAction::safe(
                            "gh.auth_login",
                            "Run `gh auth login` to authenticate the GitHub CLI.",
                        )
                        .command("gh auth login"),
                    )
            }
        }
    }
}

/// Get a GitHub personal access token using the preference cascade.
///
/// 1. `gh auth token` (preferred — respects SSO and session management).
/// 2. `GITHUB_TOKEN` environment variable.
/// 3. `GH_TOKEN` environment variable.
///
/// Returns [`Failure::blocked`] if no token is available.
pub fn get_token() -> Result<String, Failure> {
    // 1. Try `gh auth token` first.
    if let Ok(token) = try_gh_token() {
        return Ok(token);
    }

    // 2. Fall back to environment variables.
    if let Some(token) = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()) {
        return Ok(token);
    }
    if let Some(token) = std::env::var("GH_TOKEN").ok().filter(|t| !t.is_empty()) {
        return Ok(token);
    }

    // 3. Fail cleanly — name the output, don't degrade to anonymous.
    Err(Failure::blocked(
        "github.no_token",
        "no GitHub token available — cannot access private repositories",
    )
    .expected("one of: `gh auth token`, $GITHUB_TOKEN, or $GH_TOKEN")
    .actual("none of the above provided a token")
    .fix(
        FixAction::safe(
            "github.configure_auth",
            "Authenticate with `gh auth login`, or set $GITHUB_TOKEN / $GH_TOKEN.",
        )
        .command("gh auth login"),
    ))
}

/// Run `gh auth token` and return its stdout if it exits zero with content.
fn try_gh_token() -> Result<String, Error> {
    let output = Command::new("gh").args(["auth", "token"]).output();

    let output = match output {
        Ok(o) => o,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::GhFailed {
                code: -1,
                stderr: "gh not found on PATH".to_owned(),
            });
        }
        Err(e) => {
            return Err(Error::GhFailed {
                code: -1,
                stderr: e.to_string(),
            });
        }
    };

    if !output.status.success() {
        return Err(Error::GhFailed {
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr)
                .trim_end()
                .to_owned(),
        });
    }

    let token = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned();
    if token.is_empty() {
        return Err(Error::GhFailed {
            code: 0,
            stderr: "empty token output".to_owned(),
        });
    }

    Ok(token)
}

/// Rewrite a GitHub HTTPS URL to include the token from the auth cascade.
///
/// If the URL is not a GitHub HTTPS URL, returns it unchanged.
/// If no token is available, returns `Err` naming the missing output.
pub fn github_auth_url(url: &str) -> Result<String, Failure> {
    if !url.starts_with("https://github.com/") {
        return Ok(url.to_owned());
    }

    let token = get_token()?;
    let rewritten = url.replacen("https://", &format!("https://{token}@"), 1);
    Ok(rewritten)
}

/// Read a response body into a String using the ureq 3.x Response API.
fn read_body<T: Read>(mut reader: T) -> Result<String, Failure> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf).map_err(|e| {
        Failure::failed(
            "github.read_body",
            format!("could not read response body: {e}"),
        )
    })?;
    Ok(buf)
}

/// Fetch the raw tarball of a GitHub repository at a given ref.
pub fn fetch_tarball(repo: &str, r#ref: &str) -> Result<Vec<u8>, Failure> {
    let token = get_token()?;
    let url = format!("https://api.github.com/repos/{repo}/tarball/{ref}");

    let response = ureq::get(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| {
            Failure::failed(
                "github.tarball_fetch",
                format!("could not fetch tarball: {e}"),
            )
            .expected("the repository and ref to exist")
            .actual(format!("HTTP error: {e}"))
        })?;

    let status = response.status();
    let body = read_body(response.into_body().as_reader())?;

    if status != 200 {
        return Err(Failure::failed(
            "github.tarball_http",
            format!("GitHub API returned {status}"),
        )
        .expected("HTTP 200")
        .actual(body));
    }

    Ok(body.into_bytes())
}

/// Resolve a GitHub ref (branch, tag, or SHA) to a commit SHA via the API.
pub fn resolve_ref(repo: &str, r#ref: &str) -> Result<String, Failure> {
    let token = get_token()?;
    let url = format!("https://api.github.com/repos/{repo}/git/ref/heads/{ref}");

    let response = ureq::get(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .map_err(|e| {
            Failure::failed("github.resolve_ref", format!("could not resolve ref: {e}"))
                .expected(format!("{repo}/{ref} to exist"))
                .actual(format!("HTTP error: {e}"))
        })?;

    let status = response.status();
    let body = read_body(response.into_body().as_reader())?;

    if status != 200 {
        return Err(Failure::failed(
            "github.resolve_ref_http",
            format!("GitHub API returned {status}"),
        )
        .expected("HTTP 200")
        .actual(body));
    }

    let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        Failure::failed(
            "github.resolve_ref_parse",
            format!("could not parse JSON: {e}"),
        )
    })?;

    parsed["object"]["sha"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| {
            Failure::failed("github.resolve_ref_parse", "response missing object.sha")
                .expected("an object.sha field in the API response")
        })
}
