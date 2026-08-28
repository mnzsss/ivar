//! Skill source resolution: parse source strings and CLI flags into an [`ExternalRef`].
//!
//! # Supported forms
//!
//! - `owner/repo` — default branch, root of repository.
//! - `https://github.com/owner/repo` — default branch, root of repository.
//! - `https://github.com/owner/repo/tree/<ref>/<path>` — explicit git ref and subpath inside repo.
//!
//! # Rejected forms
//!
//! Non-GitHub hosts (GitLab, Bitbucket), SSH URLs (`git@github.com:...`), bare paths,
//! `@ref` suffixes (`owner/repo@ref`), and conflicting `--path` flags combined with a subpath URL.

use crate::domain::skill::ExternalRef;
use crate::error::{Failure, FixAction};

/// Parse a source argument and optional `--path` / `--ref` flags into an [`ExternalRef`].
pub fn parse_source(
    source: &str,
    path_flag: Option<&str>,
    ref_flag: Option<&str>,
) -> Result<ExternalRef, Failure> {
    let source = source.trim();

    let (repo, url_path, url_ref) = if let Some(stripped) = source.strip_prefix("https://github.com/") {
        let clean = stripped.trim_end_matches('/');
        let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() == 2 {
            let owner = parts.first().copied().unwrap_or_default();
            let raw_repo = parts.get(1).copied().unwrap_or_default();
            let repo_name = raw_repo.strip_suffix(".git").unwrap_or(raw_repo);
            (format!("{owner}/{repo_name}"), String::new(), String::new())
        } else if parts.len() >= 4 && parts.get(2) == Some(&"tree") {
            let owner = parts.first().copied().unwrap_or_default();
            let raw_repo = parts.get(1).copied().unwrap_or_default();
            let repo_name = raw_repo.strip_suffix(".git").unwrap_or(raw_repo);
            let repo = format!("{owner}/{repo_name}");
            let git_ref = parts.get(3).copied().unwrap_or_default().to_owned();
            let path = parts.get(4..).unwrap_or_default().join("/");
            (repo, path, git_ref)
        } else {
            return Err(invalid_source_failure(source));
        }
    } else if !source.contains("://") && !source.contains('@') && !source.contains(' ') {
        let clean = source.trim_end_matches('/');
        let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() == 2 {
            let owner = parts.first().copied().unwrap_or_default();
            let raw_repo = parts.get(1).copied().unwrap_or_default();
            let repo_name = raw_repo.strip_suffix(".git").unwrap_or(raw_repo);
            (format!("{owner}/{repo_name}"), String::new(), String::new())
        } else {
            return Err(invalid_source_failure(source));
        }
    } else {
        return Err(invalid_source_failure(source));
    };

    // Check for path conflict: --path flag AND a subpath in the URL
    if !url_path.is_empty() && path_flag.is_some_and(|p| !p.trim().is_empty()) {
        let flag_path = path_flag.unwrap_or_default();
        return Err(Failure::blocked(
            "skill.add.path_conflict",
            format!("cannot combine --path flag (`{flag_path}`) with a URL subpath (`{url_path}`)"),
        )
        .expected("either a URL with a subpath or the --path flag, not both")
        .actual(format!("URL path `{url_path}` and --path `{flag_path}`"))
        .fix(FixAction::safe(
            "skill.add.fix_path_conflict",
            "Specify the subpath either in the URL or via --path, not both.",
        )));
    }

    let final_path = if let Some(p) = path_flag.filter(|p| !p.trim().is_empty()) {
        p.trim_matches('/').to_owned()
    } else {
        url_path
    };

    let final_ref = if let Some(r) = ref_flag.filter(|r| !r.trim().is_empty()) {
        r.trim().to_owned()
    } else if !url_ref.is_empty() {
        url_ref
    } else {
        String::new()
    };

    Ok(ExternalRef {
        repo,
        path: final_path,
        git_ref: final_ref,
    })
}

fn invalid_source_failure(source: &str) -> Failure {
    Failure::blocked(
        "skill.add.invalid_source",
        format!("unsupported source format: `{source}`"),
    )
    .expected("owner/repo, https://github.com/owner/repo, or https://github.com/owner/repo/tree/<ref>/<path>")
    .actual(format!("`{source}`"))
    .fix(FixAction::safe(
        "skill.add.valid_format",
        "Use one of the supported source forms: owner/repo, https://github.com/owner/repo, or https://github.com/owner/repo/tree/<ref>/<path>",
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/skill/source.rs"]
mod tests;
