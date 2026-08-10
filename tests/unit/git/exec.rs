#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::{seeded_repo, utf8_temp_dir};

#[test]
fn clone_bare_produces_a_bare_repository() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");

    clone_bare(origin.as_str(), &bare).unwrap();

    // A bare clone has its object database at the top level, not under
    // `.git/` — that is what makes it a valid `--git-dir`.
    assert!(bare.join("HEAD").is_file());
    assert!(bare.join("objects").is_dir());
    assert!(!bare.join(".git").exists());
}

#[test]
fn a_failing_clone_is_refused_with_the_invocation_and_gits_diagnostic() {
    let (_guard, dir) = utf8_temp_dir();

    let error =
        clone_bare(dir.join("nowhere").as_str(), &dir.join("dest")).expect_err("nothing to clone");

    match error {
        Error::Refused { command, detail } => {
            assert!(command.starts_with("git clone --bare"), "was: {command}");
            assert!(!detail.is_empty(), "git said nothing about why");
        }
        other => panic!("expected Refused, got {other:?}"),
    }
}

#[test]
fn add_worktree_checks_out_the_branchs_content() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let worktree = dir.join("api/main");

    add_worktree(&bare, &worktree, "main").unwrap();

    assert_eq!(
        std::fs::read_to_string(worktree.join("README.md")).unwrap(),
        "seed\n"
    );
}

#[test]
fn adding_a_worktree_on_a_branch_that_does_not_exist_is_refused() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();

    let error =
        add_worktree(&bare, &dir.join("wt"), "no-such-branch").expect_err("branch does not exist");

    assert!(matches!(error, Error::Refused { .. }));
}

/// A worktree path that already holds something is git's call to refuse,
/// not this module's to pre-empt. Duplicating the check here would mean two
/// answers to one question, and git's is the one that is always right.
#[test]
fn adding_a_worktree_over_an_occupied_path_is_refused_by_git() {
    let (_guard, dir) = utf8_temp_dir();
    let origin = seeded_repo(&dir.join("origin"), "main");
    let bare = dir.join("api.bare");
    clone_bare(origin.as_str(), &bare).unwrap();
    let occupied = dir.join("occupied");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("something"), "here").unwrap();

    let error = add_worktree(&bare, &occupied, "main").expect_err("path is occupied");

    assert!(matches!(error, Error::Refused { .. }));
}
