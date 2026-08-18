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
use crate::domain::name::{FeatureName, RepoName};
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

/// Regression for MNZS-399: the hook forwards the *absolute* path the
/// executor's tool call reported, and a repo-prefixed contract like
/// `acme/src/` must still match it. The absolute path can be the path through
/// the session view dir (`.../sessions/<id>/acme/src/main.rs`) or the
/// symlink's worktree target (`.../.ivar/repos/acme/<branch>/src/main.rs`),
/// which carries a branch segment no contract ever names. Both must be
/// allowed; before the fix, matching the raw absolute path against the
/// contract denied every write.
#[test]
fn an_absolute_path_is_normalised_against_the_contract() {
    let (_guard, root) = seeded_board();
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    // Ask the guard for a workstream whose contract names a repo, the shape
    // `ensure_contracts_avoid_locked_promotions` enforces after a receipt.
    board.graph.workstreams[0].write_contract = vec!["acme/src/".to_owned()];
    board.write(&layout, &feature).unwrap();
    // Promote `acme` so the feature record carries the repo->worktree mapping
    // `normalise_path` needs to turn a worktree path back into `<repo>/<path>`.
    let mut feature_record = Feature::read(&layout, &feature).unwrap().unwrap();
    feature_record.promote(RepoName::new("acme").unwrap());
    feature_record.write(&layout).unwrap();

    let ctx = Ctx::new(root.clone());
    let view_dir = format!(
        "{}/.ivar/features/checkout/sessions/s1/acme/src/main.rs",
        root
    );
    let worktree = layout
        .repo_worktree(&RepoName::new("acme").unwrap(), &feature_record.branch)
        .join("src/main.rs");

    for path in [
        "acme/src/main.rs".to_owned(),
        view_dir,
        worktree.as_str().to_owned(),
    ] {
        let outcome = guard_check(
            &ctx,
            GuardCheckInput {
                feature: Some("checkout".to_owned()),
                session: Some("sess-src".to_owned()),
                path: Some(path.clone()),
            },
        )
        .unwrap();

        assert!(outcome.is_clean());
        assert!(
            outcome.value.allowed,
            "the normalized absolute path must be allowed: {path}"
        );
        assert_eq!(outcome.value.workstream.as_deref(), Some("ws-src"));
    }
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
