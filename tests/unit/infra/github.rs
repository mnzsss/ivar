#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

// -- token_from: the pure cascade ----------------------------------------

#[test]
fn gh_token_wins_over_env_vars() {
    let token = token_from(Some("gh_tok".to_owned()), Some("env_tok".to_owned()), None).unwrap();
    assert_eq!(token, "gh_tok");
}

#[test]
fn github_token_wins_over_gh_token() {
    let token = token_from(
        None,
        Some("github_tok".to_owned()),
        Some("gh_tok".to_owned()),
    )
    .unwrap();
    assert_eq!(token, "github_tok");
}

#[test]
fn gh_token_is_the_last_resort() {
    let token = token_from(None, None, Some("gh_tok".to_owned())).unwrap();
    assert_eq!(token, "gh_tok");
}

#[test]
fn empty_sources_are_ignored() {
    let token = token_from(Some(String::new()), None, None).unwrap_err();
    assert_eq!(token.code, "github.no_token");
}

#[test]
fn no_sources_fails_blocked_naming_the_output() {
    let error = token_from(None, None, None).unwrap_err();
    assert_eq!(error.status, crate::error::Status::Blocked);
    assert_eq!(error.code, "github.no_token");
    assert!(
        error.what.contains("gh auth token")
            || error
                .expected
                .as_deref()
                .is_some_and(|e| e.contains("GITHUB_TOKEN")),
        "failure names the expected output: {error}"
    );
}

// -- gh integration: covered by token_from's `gh` branch -----------------
//
// `try_gh_token` is a thin subprocess wrapper (spawn `gh auth token`,
// take its stdout). Driving it through a fake `gh` on PATH would require
// mutating the process environment, which this crate forbids; the
// preference order it feeds — gh first — is fully exercised by
// `token_from` above, and the spawn failure path by `Error::GhFailed`.

// -- url helpers ----------------------------------------------------------

#[test]
fn is_github_https_only_matches_github_com() {
    assert!(is_github_https("https://github.com/owner/repo"));
    assert!(!is_github_https("https://gitlab.com/owner/repo"));
    assert!(!is_github_https("git@github.com:owner/repo.git"));
    assert!(!is_github_https("/local/path/repo"));
}
