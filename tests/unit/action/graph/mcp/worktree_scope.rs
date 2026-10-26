#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use super::*;

fn hall() -> (tempfile::TempDir, Layout) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    for worktree in ["main", "feat/auth", "add-auth"] {
        let dir = root.join(".ivar/repos/api").join(worktree);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join(".git"), "gitdir: elsewhere\n").unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\n").unwrap();
    }
    std::fs::create_dir_all(root.join(".ivar/repos/api/.bare")).unwrap();
    (tmp, Layout::at(root))
}

fn worktree(reference: Option<Reference>) -> Located {
    match reference {
        Some(Reference::Worktree(located)) => located,
        other => panic!("expected a worktree, got {other:?}"),
    }
}

#[test]
fn an_absolute_worktree_path_names_its_worktree_and_the_path_inside() {
    let (_tmp, layout) = hall();
    let path = layout.root().join(".ivar/repos/api/add-auth/src/a.rs");

    let located = worktree(locate(&layout, None, path.as_str()));

    assert_eq!(
        located,
        Located {
            repo: "api".to_owned(),
            branch: "add-auth".to_owned(),
            rest: "src/a.rs".to_owned(),
            explicit: true,
        }
    );
}

#[test]
fn a_hall_relative_store_path_names_its_worktree() {
    let (_tmp, layout) = hall();

    let located = worktree(locate(&layout, None, "./.ivar/repos/api/main/src/a.rs"));

    assert_eq!(located.branch, "main");
    assert_eq!(located.rest, "src/a.rs");
}

#[test]
fn a_branch_with_a_slash_resolves_to_the_nested_worktree() {
    let (_tmp, layout) = hall();

    let located = worktree(locate(&layout, None, ".ivar/repos/api/feat/auth/src/a.rs"));

    assert_eq!(located.branch, "feat/auth");
    assert_eq!(located.rest, "src/a.rs");
}

#[test]
fn the_worktree_root_has_an_empty_inner_path() {
    let (_tmp, layout) = hall();

    let located = worktree(locate(&layout, None, ".ivar/repos/api/add-auth"));

    assert_eq!(located.rest, "");
}

#[test]
fn an_explicit_path_into_a_missing_worktree_is_reported_missing() {
    let (_tmp, layout) = hall();

    let reference = locate(&layout, None, ".ivar/repos/api/gone/src/a.rs");

    assert!(
        matches!(reference, Some(Reference::Missing { .. })),
        "{reference:?}"
    );
}

#[test]
fn paths_outside_the_store_and_bare_clones_are_not_worktree_paths() {
    let (_tmp, layout) = hall();

    assert_eq!(locate(&layout, None, "src/a.rs"), None);
    assert_eq!(locate(&layout, None, "/etc/hosts"), None);
    assert_eq!(locate(&layout, None, ".ivar/repos/api/.bare/HEAD"), None);
}

#[test]
fn a_relative_path_counts_only_when_it_exists_under_the_working_directory() {
    let (_tmp, layout) = hall();
    let cwd = layout.root().join(".ivar/repos/api/add-auth");

    let located = worktree(locate(&layout, Some(&cwd), "src/a.rs"));
    assert_eq!(located.branch, "add-auth");
    assert!(!located.explicit);

    assert_eq!(locate(&layout, Some(&cwd), "src/missing.rs"), None);
}

#[test]
fn a_path_through_a_link_into_the_store_resolves_to_the_linked_worktree() {
    let (_tmp, layout) = hall();
    let view = layout.root().join("view");
    std::fs::create_dir_all(&view).unwrap();
    std::os::unix::fs::symlink(
        layout.root().join(".ivar/repos/api/add-auth"),
        view.join("api"),
    )
    .unwrap();

    let located = worktree(locate(&layout, Some(&view), "api/src/a.rs"));

    assert_eq!(located.branch, "add-auth");
    assert_eq!(located.rest, "src/a.rs");
    assert!(!located.explicit);
}

#[test]
fn rewriting_a_token_keeps_its_line_range_and_punctuation() {
    let token = Token::parse("`/hall/.ivar/repos/api/add-auth/src/a.rs:10-20`,");

    assert_eq!(
        token.path(),
        Some("/hall/.ivar/repos/api/add-auth/src/a.rs")
    );
    assert_eq!(token.rewrite("api", "src/a.rs"), "`api/src/a.rs:10-20`,");
    assert_eq!(token.rewrite("api", ""), "`api:10-20`,");
}

#[test]
fn normalizing_resolves_dots_lexically() {
    assert_eq!(
        normalize(Utf8Path::new("/a/./b/../c/d")),
        Utf8PathBuf::from("/a/c/d")
    );
}

#[test]
fn a_query_without_worktree_paths_is_left_alone() {
    let (_tmp, layout) = hall();
    let db = GraphDb::open_in_memory().unwrap();

    let scoped = scope_query(&db, &layout, None, "session enforcement src/a.rs");

    let Scoped::Run(scope) = scoped else {
        panic!("expected a run, got {scoped:?}");
    };
    assert_eq!(scope.query, "session enforcement src/a.rs");
    assert!(scope.labels.is_empty());
}

#[test]
fn an_explicit_path_into_an_unpromoted_worktree_is_refused_as_not_indexed() {
    let (_tmp, layout) = hall();
    let db = GraphDb::open_in_memory().unwrap();
    let path = layout.root().join(".ivar/repos/api/add-auth/src/a.rs");

    let scoped = scope_query(&db, &layout, None, path.as_str());

    let Scoped::Refused(answer) = scoped else {
        panic!("expected a refusal, got {scoped:?}");
    };
    assert!(answer.contains("not indexed"), "{answer}");
    assert!(answer.contains("add-auth"), "{answer}");
}

#[test]
fn an_explicit_path_into_a_missing_worktree_is_refused() {
    let (_tmp, layout) = hall();
    let db = GraphDb::open_in_memory().unwrap();

    let scoped = scope_query(&db, &layout, None, ".ivar/repos/api/gone/src/a.rs");

    let Scoped::Refused(answer) = scoped else {
        panic!("expected a refusal, got {scoped:?}");
    };
    assert!(answer.contains("No worktree exists"), "{answer}");
}
