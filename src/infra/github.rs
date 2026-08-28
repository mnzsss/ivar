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
    token_from(
        try_gh_token().ok(),
        std::env::var("GITHUB_TOKEN").ok(),
        std::env::var("GH_TOKEN").ok(),
    )
}

/// The preference cascade as a pure function: `gh` wins, then `GITHUB_TOKEN`,
/// then `GH_TOKEN`; nothing resolves to [`Failure::blocked`] naming the
/// output. Pure so the ordering and the no-token failure are testable without
/// touching the process environment (which the crate forbids mutating).
fn token_from(
    gh: Option<String>,
    github_token: Option<String>,
    gh_token: Option<String>,
) -> Result<String, Failure> {
    let gh = gh.filter(|t| !t.is_empty());
    let github_token = github_token.filter(|t| !t.is_empty());
    let gh_token = gh_token.filter(|t| !t.is_empty());

    if let Some(token) = gh {
        return Ok(token);
    }
    if let Some(token) = github_token {
        return Ok(token);
    }
    if let Some(token) = gh_token {
        return Ok(token);
    }

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

/// Whether `url` is a GitHub HTTPS URL — the only host the auth cascade
/// serves. Anything else (SSH, a mirror, a bare path) is left alone.
#[must_use]
pub fn is_github_https(url: &str) -> bool {
    url.starts_with("https://github.com/")
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
    let mut bytes = Vec::new();
    response
        .into_body()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| {
            Failure::failed(
                "github.read_bytes",
                format!("could not read response bytes: {e}"),
            )
        })?;

    if status != 200 {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        return Err(Failure::failed(
            "github.tarball_http",
            format!("GitHub API returned {status}"),
        )
        .expected("HTTP 200")
        .actual(text));
    }

    Ok(bytes)
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

    parsed
        .get("object")
        .and_then(|object| object.get("sha"))
        .and_then(serde_json::Value::as_str)
        .map(String::from)
        .ok_or_else(|| {
            Failure::failed("github.resolve_ref_parse", "response missing object.sha")
                .expected("an object.sha field in the API response")
        })
}

#[cfg(test)]
#[path = "../../tests/unit/infra/github.rs"]
mod tests;
