#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::execute::prepare::{self as prepare_action, PrepareInput as PrepareActionInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

const GRAPH_JSON: &str = r#"{
    "workstreams": [
        {
            "id": "ws-src",
            "title": "Source files",
            "operations": ["write-code"],
            "depends_on": [],
            "write_contract": ["src/"],
            "provider": "claude-code"
        },
        {
            "id": "ws-docs",
            "title": "Docs",
            "operations": ["write-docs"],
            "depends_on": [],
            "write_contract": ["docs/"],
            "provider": "claude-code"
        }
    ]
}"#;

/// A plan that backs `GRAPH_JSON`. `prepare` refuses a graph whose
/// operations the plan does not document, so the scaffolded plan
/// `plan create` writes is not enough to seed a board with.
const PLAN_TEXT: &str = r#"# Plan

## Operations

### ws-src
- write-code
write_contract:
- src/

### ws-docs
- write-docs
write_contract:
- docs/

## Operation details

**write-code** — Implement write-code.

**write-docs** — Implement write-docs.
"#;

/// A hall with a prepared board, sessions injected manually.
fn seeded_board() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    fs::write_text(&root.join("plans/checkout/plan.md"), PLAN_TEXT).unwrap();

    let graph = root.join("graph.json");
    fs::write_text(&graph, GRAPH_JSON).unwrap();
    prepare_action::prepare(
        &ctx,
        PrepareActionInput {
            feature: "checkout".to_owned(),
            graph_json: graph.to_string(),
            session: None,
        },
    )
    .unwrap();

    // Inject sessions into the board.
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    board
        .sessions
        .insert("sess-src".to_owned(), "ws-src".to_owned());
    board
        .sessions
        .insert("sess-docs".to_owned(), "ws-docs".to_owned());
    board.write(&layout, &feature).unwrap();

    (guard, root)
}

#[test]
fn path_inside_the_workstream_contract_is_allowed() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    let outcome = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            path: Some("src/main.rs".to_owned()),
        },
    )
    .unwrap();

    assert!(outcome.is_clean());
    assert!(outcome.value.allowed);
    assert_eq!(outcome.value.workstream.as_deref(), Some("ws-src"));
}

#[test]
fn path_outside_the_workstream_contract_is_denied_naming_the_workstream() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    let outcome = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            path: Some("docs/guide.md".to_owned()),
        },
    )
    .unwrap();

    assert!(outcome.is_clean());
    assert!(!outcome.value.allowed);
    assert_eq!(outcome.value.workstream.as_deref(), Some("ws-src"));
}

#[test]
fn unknown_session_is_never_allowed() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    let outcome = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-ghost".to_owned()),
            path: Some("src/main.rs".to_owned()),
        },
    )
    .unwrap();

    assert!(outcome.is_clean());
    assert!(!outcome.value.allowed);
    assert!(outcome.value.workstream.is_none());
}

#[test]
fn dot_dot_does_not_escape_the_contract() {
    let (_guard, root) = seeded_board();
    let ctx = Ctx::new(root.clone());

    // ".." in path should be rejected by WriteContract::allows.
    let outcome = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            path: Some("../outside.txt".to_owned()),
        },
    )
    .unwrap();

    assert!(outcome.is_clean());
    assert!(!outcome.value.allowed);
}

#[test]
fn missing_feature_argument_returns_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    let failure = guard_check(
        &ctx,
        GuardCheckInput {
            feature: None,
            session: Some("sess-src".to_owned()),
            path: Some("src/main.rs".to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, crate::error::Status::Blocked);
    assert_eq!(failure.code, "execute.guard_check.missing_feature");
}

#[test]
fn missing_session_argument_returns_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    let failure = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: None,
            path: Some("src/main.rs".to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, crate::error::Status::Blocked);
    assert_eq!(failure.code, "execute.guard_check.missing_session");
}

#[test]
fn missing_path_argument_returns_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());

    let failure = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            path: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, crate::error::Status::Blocked);
    assert_eq!(failure.code, "execute.guard_check.missing_path");
}

#[test]
fn absent_board_returns_denied_with_no_workstream() {
    // A hall that exists but whose board was never prepared: the guard
    // must deny by default — an unreadable board is a denial, never a
    // grant.
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let outcome = guard_check(
        &ctx,
        GuardCheckInput {
            feature: Some("checkout".to_owned()),
            session: Some("sess-src".to_owned()),
            path: Some("src/main.rs".to_owned()),
        },
    )
    .unwrap();

    assert!(outcome.is_clean());
    assert!(!outcome.value.allowed);
    assert!(outcome.value.workstream.is_none());
}

// -- the path the hook forwards: an absolute worktree path --------------
//
// The guard hook forwards the path the executor handed its tool. OpenCode and
// Claude Code resolve the view dir's per-repo symlink to the real worktree,
// `<hall>/.ivar/repos/<repo>/<branch>/<path>`, while the write contract names
// its files `<repo>/<path>` — the shape `tick::launch::audit_path` writes.
// `contract_path_allows` must relativize the absolute worktree path back into
// that shape, or every Write/Edit is denied.

/// A contract names its file `<repo>/<path>`; the hook forwards the real
/// worktree's absolute path with the branch segment in the middle. It must be
/// allowed.
#[test]
fn an_absolute_worktree_path_is_allowed_in_the_repo_s_shape() {
    let contract = WriteContract::new(vec![
        "gaio-backend/packages/console/src/workflows/repositories/workflow.ts".to_owned(),
    ]);
    let repo = RepoName::new("gaio-backend").unwrap();
    let worktree = Utf8PathBuf::from("/hall/.ivar/repos/gaio-backend/feat/auth");
    let resolved = worktree.join("packages/console/src/workflows/repositories/workflow.ts");

    let worktrees = [(repo, worktree.clone())];
    assert!(
        contract_path_allows(&contract, &resolved, &worktrees),
        "a workstream's own contracted file, addressed by the worktree's absolute path, must be allowed"
    );
}

/// The same absolute worktree path, but to a sibling file the repo-prefixed
/// contract does not name, is denied.
#[test]
fn an_absolute_worktree_path_outside_the_contract_is_denied() {
    let contract = WriteContract::new(vec![
        "gaio-backend/packages/console/src/workflows/repositories/workflow.ts".to_owned(),
    ]);
    let repo = RepoName::new("gaio-backend").unwrap();
    let worktree = Utf8PathBuf::from("/hall/.ivar/repos/gaio-backend/feat/auth");
    let resolved = worktree.join("packages/console/src/workflows/controllers/controller.ts");

    let worktrees = [(repo, worktree)];
    assert!(!contract_path_allows(&contract, &resolved, &worktrees));
}

/// A path that is not under any promoted worktree — the session view dir's
/// own symlink shape, or a bare relative path — is matched as resolved,
/// exactly as the guard behaved before the relativization existed.
#[test]
fn a_path_not_under_any_worktree_is_matched_as_resolved() {
    let contract = WriteContract::new(vec!["src/".to_owned()]);
    let resolved = Utf8PathBuf::from("/hall/src/main.rs");

    assert!(contract_path_allows(&contract, &resolved, &[]));
}
