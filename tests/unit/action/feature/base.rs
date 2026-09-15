#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::git::{Git, System};
use crate::test_support::{git, seeded_repo, utf8_temp_dir};

/// A bare clone with a `main` worktree and a `feat/x` worktree carrying two
/// commits that `main` does not have.
fn bare_with_feature_commits() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    System.clone_bare(origin.as_str(), &bare).unwrap();
    git(&bare, &["branch", "feat/x"]);
    let feature = dir.join("feat-x");
    System.add_worktree(&bare, &feature, "feat/x").unwrap();
    for name in ["one", "two"] {
        std::fs::write(feature.join(format!("{name}.md")), format!("{name}\n")).unwrap();
        git(&feature, &["add", "."]);
        git(&feature, &["commit", "-m", name]);
    }
    let main = dir.join("main");
    System.add_worktree(&bare, &main, "main").unwrap();
    (guard, bare, main)
}

fn commit_on_main(main: &Utf8PathBuf, name: &str) {
    std::fs::write(main.join(format!("{name}.md")), format!("{name}\n")).unwrap();
    git(main, &["add", "."]);
    git(main, &["commit", "-m", name]);
}

#[test]
fn an_unmerged_branch_counts_its_commits() {
    let (_guard, bare, main) = bare_with_feature_commits();
    commit_on_main(&main, "other");

    assert_eq!(
        unmerged_commits(&System, &bare, "main", "feat/x").unwrap(),
        2
    );
}

#[test]
fn a_squash_merged_branch_is_delivered() {
    let (_guard, bare, main) = bare_with_feature_commits();
    commit_on_main(&main, "other");
    git(&main, &["merge", "--squash", "feat/x"]);
    git(&main, &["commit", "-m", "squash"]);
    commit_on_main(&main, "later");

    assert_eq!(
        unmerged_commits(&System, &bare, "main", "feat/x").unwrap(),
        0
    );
}

#[test]
fn a_rebase_landed_branch_is_delivered() {
    let (_guard, bare, main) = bare_with_feature_commits();
    commit_on_main(&main, "other");
    git(&main, &["cherry-pick", "main..feat/x"]);

    assert_eq!(
        unmerged_commits(&System, &bare, "main", "feat/x").unwrap(),
        0
    );
}
