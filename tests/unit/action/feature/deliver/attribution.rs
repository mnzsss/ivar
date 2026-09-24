use super::fixture::*;
use super::*;
use crate::action::feature::deliver::attribution as deliver_attribution;
fn commit_on_checkout(root: &Utf8Path, message: &str) {
    let worktree = Layout::at(root.to_path_buf()).repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(worktree.join("more.md"), "more\n").unwrap();
    git(&worktree, &["add", "more.md"]);
    git(&worktree, &["commit", "-m", message]);
}

#[test]
fn recognises_claude_attribution_lines_only() {
    for line in [
        "🤖 Generated with [Claude Code](https://claude.com/claude-code)",
        "Generated with Claude Code",
        "Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
        "co-authored-by: bot <noreply@anthropic.com>",
    ] {
        assert!(
            deliver_attribution::is_attribution(line),
            "should flag: {line}"
        );
    }
    for line in [
        "Co-Authored-By: Jane Doe <jane@example.com>",
        "docs: explain how Claude Code settings are merged",
        "fix: keep the 🤖 emoji in release notes",
    ] {
        assert!(
            !deliver_attribution::is_attribution(line),
            "should pass: {line}"
        );
    }
}

#[test]
fn preview_refuses_a_commit_carrying_claude_attribution() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    commit_on_checkout(
        &root,
        "feat: more\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
    );

    let failure = deliver(&Ctx::new(root.clone()), preview_input("checkout")).unwrap_err();

    assert_eq!(failure.code, "deliver.ai_attribution");
    let failure = deliver(&Ctx::new(root), land_preview_input("checkout")).unwrap_err();
    assert_eq!(failure.code, "deliver.ai_attribution");
}

#[test]
fn apply_refuses_a_body_carrying_claude_attribution() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let mut input = apply_input("checkout", "whatever");
    input.global_metadata.body = Some(
        "Adds more.\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)".to_owned(),
    );

    let failure = deliver(&Ctx::new(root), input).unwrap_err();

    assert_eq!(failure.code, "deliver.ai_attribution");
}

#[test]
fn preview_accepts_a_feature_without_attribution() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    commit_on_checkout(
        &root,
        "feat: more\n\nCo-Authored-By: Jane Doe <jane@example.com>",
    );

    assert!(deliver(&Ctx::new(root), preview_input("checkout")).is_ok());
}
