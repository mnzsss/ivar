#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::process::Command;

use camino::Utf8PathBuf;
use tempfile::{TempDir, tempdir};

use crate::action::graph::provenance::{RepoIndexState, collect, index_notice, render};
use crate::action::graph::session::{RepoViewInfo, SessionView};
use crate::store::graph::db::GraphDb;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn state(indexed: Option<&str>, head: Option<&str>, behind: usize) -> RepoIndexState {
    RepoIndexState {
        repo: "core".into(),
        indexed: indexed.map(str::to_owned),
        head: head.map(str::to_owned),
        behind_upstream: behind,
    }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn commit(cwd: &Path, file: &str) -> String {
    std::fs::write(cwd.join(file), file).unwrap();
    git(cwd, &["add", "."]);
    git(cwd, &["commit", "-m", file]);
    git(cwd, &["rev-parse", "HEAD"])
}

/// A bare `origin`, plus a clone of it with one commit pushed.
fn clone_fixture() -> (TempDir, Utf8PathBuf, Utf8PathBuf, String) {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let origin = root.join("origin.git");
    let checkout = root.join("core");
    std::fs::create_dir_all(&origin).unwrap();
    git(origin.as_std_path(), &["init", "--bare", "-b", "main"]);
    git(
        root.as_std_path(),
        &["clone", origin.as_str(), checkout.as_str()],
    );
    git(checkout.as_std_path(), &["checkout", "-B", "main"]);
    let first = commit(checkout.as_std_path(), "one.txt");
    git(checkout.as_std_path(), &["push", "-u", "origin", "main"]);
    (tmp, origin, checkout, first)
}

fn view_of(checkout: &Utf8PathBuf) -> SessionView {
    SessionView::Base {
        repos: vec![RepoViewInfo {
            repo_name: "core".into(),
            worktree_path: checkout.clone(),
            is_layer: false,
            base_commit: None,
        }],
    }
}

fn db_indexed_at(checkout: &Utf8PathBuf, commit: Option<&str>) -> GraphDb {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("core", checkout.as_str(), "main", commit)
        .unwrap();
    db
}

#[test]
fn a_current_index_names_its_commit_and_raises_no_warning() {
    let notice = render(&[state(Some(A), Some(A), 0)]).unwrap();

    assert_eq!(notice, "Graph index commits: core@aaaaaaa.");
}

#[test]
fn an_index_behind_the_checkout_head_warns_with_both_commits() {
    let notice = render(&[state(Some(A), Some(B), 0)]).unwrap();

    assert!(
        notice.starts_with("Graph index commits: core@aaaaaaa."),
        "{notice}"
    );
    assert!(notice.contains("index is at aaaaaaa"), "{notice}");
    assert!(notice.contains("checkout HEAD is bbbbbbb"), "{notice}");
    assert!(notice.contains("refresh_index"), "{notice}");
}

#[test]
fn a_repo_without_an_indexed_commit_is_flagged() {
    let notice = render(&[state(None, Some(B), 0)]).unwrap();

    assert!(notice.contains("core@none"), "{notice}");
    assert!(notice.contains("no indexed commit"), "{notice}");
}

#[test]
fn a_checkout_behind_its_upstream_warns_with_the_gap() {
    let one = render(&[state(Some(A), Some(A), 1)]).unwrap();
    let many = render(&[state(Some(A), Some(A), 3)]).unwrap();

    assert!(one.contains("1 commit behind its upstream"), "{one}");
    assert!(many.contains("3 commits behind its upstream"), "{many}");
    assert!(many.contains("Fetch"), "{many}");
}

#[test]
fn an_unreadable_checkout_is_not_reported_as_stale() {
    let notice = render(&[state(Some(A), None, 0)]).unwrap();

    assert_eq!(notice, "Graph index commits: core@aaaaaaa.");
}

#[test]
fn a_view_without_repos_has_no_notice() {
    assert_eq!(render(&[]), None);
}

#[test]
fn collect_reads_the_indexed_commit_and_the_checkout_head() {
    let (_tmp, _origin, checkout, first) = clone_fixture();
    let db = db_indexed_at(&checkout, Some(&first));

    let notice = index_notice(&db, &crate::git::System, &view_of(&checkout)).unwrap();

    assert_eq!(
        notice,
        format!("Graph index commits: core@{}.", &first[..7])
    );
}

#[test]
fn an_index_left_behind_by_a_new_commit_is_reported() {
    let (_tmp, _origin, checkout, first) = clone_fixture();
    let db = db_indexed_at(&checkout, Some(&first));
    let second = commit(checkout.as_std_path(), "two.txt");

    let states = collect(&db, &crate::git::System, &view_of(&checkout));

    assert_eq!(states[0].indexed.as_deref(), Some(first.as_str()));
    assert_eq!(states[0].head.as_deref(), Some(second.as_str()));
    let notice = render(&states).unwrap();
    assert!(
        notice.contains(&format!("checkout HEAD is {}", &second[..7])),
        "{notice}"
    );
}

#[test]
fn a_fetched_but_unmerged_upstream_commit_is_reported_as_behind() {
    let (tmp, origin, checkout, first) = clone_fixture();
    let db = db_indexed_at(&checkout, Some(&first));
    let other = Utf8PathBuf::from_path_buf(tmp.path().join("other")).unwrap();
    git(tmp.path(), &["clone", origin.as_str(), other.as_str()]);
    commit(other.as_std_path(), "two.txt");
    git(other.as_std_path(), &["push", "origin", "main"]);

    let before_fetch = collect(&db, &crate::git::System, &view_of(&checkout));
    git(checkout.as_std_path(), &["fetch", "origin"]);
    let after_fetch = collect(&db, &crate::git::System, &view_of(&checkout));

    assert_eq!(
        before_fetch[0].behind_upstream, 0,
        "no network without a fetch"
    );
    assert_eq!(after_fetch[0].behind_upstream, 1);
    assert!(
        render(&after_fetch)
            .unwrap()
            .contains("1 commit behind its upstream")
    );
}

#[test]
fn a_checkout_without_an_upstream_is_not_behind_anything() {
    let tmp = tempdir().unwrap();
    git(tmp.path(), &["init", "-b", "main"]);
    let head = commit(tmp.path(), "one.txt");
    let checkout = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let db = db_indexed_at(&checkout, Some(&head));

    let states = collect(&db, &crate::git::System, &view_of(&checkout));

    assert_eq!(states[0].behind_upstream, 0);
}

#[test]
fn a_feature_layer_reports_the_head_it_was_indexed_at() {
    let tmp = tempdir().unwrap();
    git(tmp.path(), &["init", "-b", "main"]);
    let base = commit(tmp.path(), "one.txt");
    let checkout = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let db = db_indexed_at(&checkout, Some(&base));
    let layer = db
        .ensure_layer_record("payments", "core", checkout.as_str(), &base)
        .unwrap();
    db.update_layer_fingerprint_and_head(layer, "fp", Some(&base))
        .unwrap();
    let view = SessionView::FeatureSession {
        feature_name: "payments".into(),
        session_id: None,
        repos: vec![RepoViewInfo {
            repo_name: "core".into(),
            worktree_path: checkout.clone(),
            is_layer: true,
            base_commit: Some(base.clone()),
        }],
    };
    let moved = commit(tmp.path(), "two.txt");

    let states = collect(&db, &crate::git::System, &view);

    assert_eq!(states[0].indexed.as_deref(), Some(base.as_str()));
    assert_eq!(states[0].head.as_deref(), Some(moved.as_str()));
}
