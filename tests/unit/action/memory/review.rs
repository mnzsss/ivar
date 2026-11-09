#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;

use crate::action::Ctx;
use crate::action::feature::create::{CreateInput, create};
use crate::action::hall::{self, InitInput};
use crate::action::memory::events::{MemoryEventScope, record_memory_event};
use crate::action::memory::review::{analysis_queried, review_in};
use crate::domain::memory_review::GapKind;
use crate::domain::name::FeatureName;
use crate::domain::observation::{Importance, Observation, SourceEvent, ToolEffect};
use crate::domain::sanitize::{SanitizeContext, Sanitized};
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observations::insert_batch;
use crate::store::memory::usage::{QueryRecord, record_query};
use crate::test_support::hall_root;

const SESSION: &str = "s1";

fn hall_with_approved_analysis() -> (tempfile::TempDir, Layout) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: camino::Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis"] {
        crate::action::plan::approve::approve(
            &ctx,
            crate::action::plan::approve::ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
    (guard, Layout::at(root))
}

/// A session of `checkout` whose only captured trace is an update page, so
/// the feature has a session but no `session.tool` memory query.
fn session_that_wrote_an_update(layout: &Layout) {
    let scope = MemoryEventScope {
        session_id: SESSION.to_owned(),
        feature: "checkout".to_owned(),
        parent_feature: String::new(),
    };
    record_memory_event(
        layout,
        &scope,
        SourceEvent::MemoryPageWritten,
        &json!({"path": "docs/memory/update/1-x.md", "kind": "update", "action": "written", "sha256": "h"}),
    )
    .unwrap();
}

fn captured_approval(layout: &Layout, at: &str) {
    let observation = Observation {
        event_id: "approval".to_owned(),
        source_event: "session.tool".to_owned(),
        occurred_at: at.to_owned(),
        session_id: SESSION.to_owned(),
        native_session_id: "n1".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0".to_owned(),
        feature: "checkout".to_owned(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::session_tool(ToolEffect::Reads),
        attempt_id: String::new(),
        body: json!({
            "tool": "Bash",
            "input": {"command": "ivar plan approve checkout analysis"},
            "result": ""
        })
        .to_string(),
    };
    let ctx = SanitizeContext::new(layout.root().to_path_buf(), Vec::new());
    let mut db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    insert_batch(&mut db, &[Sanitized::sanitize(observation, &ctx).unwrap()]).unwrap();
}

fn mcp_query(layout: &Layout, session: &str) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    record_query(
        db.conn(),
        &QueryRecord {
            source: "mcp".to_owned(),
            session: Some(session.to_owned()),
            query: "locks".to_owned(),
            filters: "{}".to_owned(),
            page_count: 1,
            observation_count: 0,
            duration_ms: 1,
            error: false,
        },
    )
    .unwrap();
}

fn gap_kinds(layout: &Layout) -> Vec<GapKind> {
    review_in(layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .value
        .gaps
        .iter()
        .map(|gap| gap.kind)
        .collect()
}

#[test]
fn a_query_served_through_mcp_before_the_approval_is_not_a_gap() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    captured_approval(&layout, "2099-01-01T00:00:00Z");
    mcp_query(&layout, SESSION);

    assert!(gap_kinds(&layout).is_empty());
}

#[test]
fn a_query_served_through_mcp_without_a_captured_approval_is_not_a_gap() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    mcp_query(&layout, SESSION);

    assert!(gap_kinds(&layout).is_empty());
}

#[test]
fn a_query_served_through_mcp_after_the_approval_is_still_a_gap() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    captured_approval(&layout, "2000-01-01T00:00:00Z");
    mcp_query(&layout, SESSION);

    assert_eq!(gap_kinds(&layout), [GapKind::AnalysisWithoutQuery]);
}

#[test]
fn a_query_of_another_session_is_not_the_features() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    captured_approval(&layout, "2099-01-01T00:00:00Z");
    mcp_query(&layout, "other");

    assert_eq!(gap_kinds(&layout), [GapKind::AnalysisWithoutQuery]);
}

#[test]
fn analysis_queried_reads_the_feature_sessions_queries() {
    let (_guard, layout) = hall_with_approved_analysis();
    let name = FeatureName::new("checkout").unwrap();
    session_that_wrote_an_update(&layout);
    assert_eq!(analysis_queried(&layout, &name), Some(false));

    mcp_query(&layout, SESSION);

    assert_eq!(analysis_queried(&layout, &name), Some(true));
}

#[test]
fn analysis_queried_is_unknown_without_a_memory_database() {
    let (_guard, layout) = hall_with_approved_analysis();
    let _ = std::fs::remove_file(layout.memory_db());
    assert_eq!(
        analysis_queried(&layout, &FeatureName::new("checkout").unwrap()),
        None
    );
}
