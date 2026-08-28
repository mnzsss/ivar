#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn parse_owner_repo_shorthand() {
    let res = parse_source("vercel-labs/skills", None, None).unwrap();
    assert_eq!(res.repo, "vercel-labs/skills");
    assert_eq!(res.path, "");
    assert_eq!(res.git_ref, "");
}

#[test]
fn parse_https_github_url() {
    let res = parse_source("https://github.com/vercel-labs/skills", None, None).unwrap();
    assert_eq!(res.repo, "vercel-labs/skills");
    assert_eq!(res.path, "");
    assert_eq!(res.git_ref, "");

    let res_dot_git =
        parse_source("https://github.com/vercel-labs/skills.git", None, None).unwrap();
    assert_eq!(res_dot_git.repo, "vercel-labs/skills");
}

#[test]
fn parse_https_github_tree_url() {
    let res = parse_source(
        "https://github.com/vercel-labs/skills/tree/main/skills/find-by-name",
        None,
        None,
    )
    .unwrap();
    assert_eq!(res.repo, "vercel-labs/skills");
    assert_eq!(res.path, "skills/find-by-name");
    assert_eq!(res.git_ref, "main");
}

#[test]
fn ref_flag_overrides_tree_url_ref() {
    let res = parse_source(
        "https://github.com/vercel-labs/skills/tree/main/skills/find-by-name",
        None,
        Some("v1.0.0"),
    )
    .unwrap();
    assert_eq!(res.repo, "vercel-labs/skills");
    assert_eq!(res.path, "skills/find-by-name");
    assert_eq!(res.git_ref, "v1.0.0");
}

#[test]
fn path_flag_with_shorthand_or_repo_url() {
    let res = parse_source("vercel-labs/skills", Some("skills/find-by-name"), None).unwrap();
    assert_eq!(res.repo, "vercel-labs/skills");
    assert_eq!(res.path, "skills/find-by-name");
    assert_eq!(res.git_ref, "");
}

#[test]
fn path_flag_conflicts_with_tree_url_subpath() {
    let err = parse_source(
        "https://github.com/vercel-labs/skills/tree/main/skills/find-by-name",
        Some("skills/other"),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, "skill.add.path_conflict");
}

#[test]
fn rejects_non_github_hosts_and_bad_formats() {
    assert_eq!(
        parse_source("https://gitlab.com/owner/repo", None, None)
            .unwrap_err()
            .code,
        "skill.add.invalid_source"
    );
    assert_eq!(
        parse_source("git@github.com:owner/repo.git", None, None)
            .unwrap_err()
            .code,
        "skill.add.invalid_source"
    );
    assert_eq!(
        parse_source("owner/repo@main", None, None)
            .unwrap_err()
            .code,
        "skill.add.invalid_source"
    );
    assert_eq!(
        parse_source("just-a-string", None, None).unwrap_err().code,
        "skill.add.invalid_source"
    );
}
