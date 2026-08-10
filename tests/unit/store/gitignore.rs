#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::utf8_temp_dir;

fn read(layout: &Layout) -> String {
    fs::read_text(&layout.gitignore_path()).unwrap().unwrap()
}

#[test]
fn an_absent_gitignore_is_created_with_exactly_the_halls_lines() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    assert_eq!(ensure(&layout).unwrap(), Changed::Created);

    assert_eq!(
        read(&layout),
        ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n\
             .claude/commands/ivar-*.md\n.opencode/commands/ivar-*.md\n"
    );
}

#[test]
fn an_existing_gitignore_keeps_its_content_and_gains_the_missing_lines() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(&layout.gitignore_path(), "node_modules/\ndist/\n").unwrap();

    assert_eq!(ensure(&layout).unwrap(), Changed::Yes);

    let content = read(&layout);
    assert!(content.starts_with("node_modules/\ndist/\n"));
    assert!(content.contains(".ivar/*"));
}

/// `sync` runs after every `git pull`. Reporting "changed" — or worse,
/// rewriting the file — when nothing was missing would put a spurious
/// modification in `git status` on every run.
#[test]
fn a_second_call_changes_nothing_and_says_so() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    ensure(&layout).unwrap();
    let after_first = fs::read_bytes(&layout.gitignore_path()).unwrap().unwrap();

    assert_eq!(ensure(&layout).unwrap(), Changed::No);

    assert_eq!(
        fs::read_bytes(&layout.gitignore_path()).unwrap().unwrap(),
        after_first
    );
}

/// Without the guard, `.ivar/*` would be glued onto `node_modules/`,
/// producing one pattern that ignores neither.
#[test]
fn a_file_with_no_trailing_newline_does_not_get_its_last_line_glued() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(&layout.gitignore_path(), "node_modules/").unwrap();

    ensure(&layout).unwrap();

    let content = read(&layout);
    assert!(
        content.lines().any(|line| line == "node_modules/"),
        "was: {content:?}"
    );
    assert!(content.lines().any(|line| line == ".ivar/*"));
}

#[test]
fn only_the_missing_lines_are_added() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(&layout.gitignore_path(), ".ivar/*\n").unwrap();

    assert_eq!(ensure(&layout).unwrap(), Changed::Yes);

    let content = read(&layout);
    assert_eq!(content.matches(".ivar/*").count(), 1);
    assert!(content.contains("!.ivar/skills/"));
    assert!(content.contains("!.ivar/setups/"));
    assert!(content.contains(".claude/commands/ivar-*.md"));
    assert!(content.contains(".opencode/commands/ivar-*.md"));
}
