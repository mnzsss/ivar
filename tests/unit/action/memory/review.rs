#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;

use crate::action::Ctx;
use crate::action::feature::create::{CreateInput, create};
use crate::action::hall::{self, InitInput};
use crate::action::memory::events::{MemoryEventScope, record_memory_event};
use crate::action::memory::review::{analysis_queried, review_in, review_read_only_in};
use crate::action::plan::approve::ApproveOutcome;
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
    let (guard, layout) = hall_with_approved_requirements();
    approve(&layout, "analysis");
    (guard, layout)
}

fn approve(layout: &Layout, gate: &str) -> crate::error::Report<ApproveOutcome> {
    crate::action::plan::approve::approve(
        &Ctx::new(layout.root().to_path_buf()),
        crate::action::plan::approve::ApproveInput {
            feature: "checkout".to_owned(),
            gate: gate.to_owned(),
        },
    )
    .unwrap()
}

fn hall_with_approved_requirements() -> (tempfile::TempDir, Layout) {
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
    let layout = Layout::at(root);
    approve(&layout, "requirements");
    (guard, layout)
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
fn analysis_queried_does_not_wait_for_a_held_memory_lock() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    crate::action::memory::drain::drain(&layout).unwrap();
    let held = crate::infra::fs::lock_exclusive(&layout.memory_lock()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = layout.clone();
    std::thread::spawn(move || {
        let queried = analysis_queried(&reader, &FeatureName::new("checkout").unwrap());
        tx.send(queried).unwrap();
    });
    let queried = rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .expect("analysis_queried must not wait for memory.lock");
    drop(held);
    assert_eq!(queried, Some(false));
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

#[test]
fn approving_analysis_without_a_feature_query_notes_it_without_a_warning() {
    let (_guard, layout) = hall_with_approved_requirements();
    session_that_wrote_an_update(&layout);
    drop(MemoryDb::open(layout.memory_db().as_std_path()).unwrap());

    let report = approve(&layout, "analysis");

    assert!(report.value.analysis_without_query);
    assert!(report.warnings.is_empty());
}

fn unattributed_query_at(layout: &Layout, ts: Option<i64>) {
    mcp_query_without_session(layout);
    if let Some(ts) = ts {
        let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn()
            .execute("UPDATE memory_usage SET ts = ?1", [ts])
            .unwrap();
    }
}

fn mcp_query_without_session(layout: &Layout) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    record_query(
        db.conn(),
        &QueryRecord {
            source: "mcp".to_owned(),
            session: None,
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

fn report(layout: &Layout) -> crate::action::memory::review::ReviewReport {
    review_in(layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .value
}

#[test]
fn a_query_with_no_session_in_the_window_is_reported_but_the_gap_stays() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    unattributed_query_at(&layout, None);

    let report = report(&layout);

    assert_eq!(report.unattributed.queries, 1);
    assert_eq!(report.gaps.len(), 1, "{:?}", report.gaps);
    let gap = report.gaps.first().unwrap();
    assert_eq!(gap.kind, GapKind::AnalysisWithoutQuery);
    assert!(
        gap.detail.contains("1 unattributed memory_query"),
        "{gap:?}"
    );
    assert!(report.written.iter().all(|page| page.kind == "update"));
}

#[test]
fn a_query_with_no_session_outside_the_window_is_not_counted() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    unattributed_query_at(&layout, Some(1));

    let report = report(&layout);

    assert_eq!(report.unattributed.queries, 0);
    assert_eq!(report.gaps.len(), 1, "{:?}", report.gaps);
    let gap = report.gaps.first().unwrap();
    assert!(!gap.detail.contains("unattributed"), "{gap:?}");
    let json = serde_json::to_value(&report).unwrap();
    assert!(json.get("unattributed").is_none(), "{json}");
}

#[test]
fn a_query_with_no_session_after_the_captured_approval_does_not_soften_the_gap() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    captured_approval(&layout, "2000-01-01T00:00:00Z");
    unattributed_query_at(&layout, None);

    let report = report(&layout);

    assert_eq!(report.unattributed.queries, 1);
    assert_eq!(report.gaps.len(), 1, "{:?}", report.gaps);
    let gap = report.gaps.first().unwrap();
    assert!(!gap.detail.contains("unattributed"), "{gap:?}");
}

#[test]
fn a_page_written_with_no_session_or_feature_is_listed_apart_from_written() {
    let (_guard, layout) = hall_with_approved_analysis();
    let nobody = MemoryEventScope::default();
    record_memory_event(
        &layout,
        &nobody,
        SourceEvent::MemoryPageWritten,
        &json!({"path": "docs/memory/update/2-y.md", "kind": "update", "action": "written", "sha256": "h"}),
    )
    .unwrap();

    let report = report(&layout);

    assert!(report.written.is_empty(), "{:?}", report.written);
    let paths: Vec<&str> = report
        .unattributed
        .written
        .iter()
        .map(|page| page.path.as_str())
        .collect();
    assert_eq!(paths, ["docs/memory/update/2-y.md"]);
    let update = report
        .gaps
        .iter()
        .find(|gap| gap.kind == GapKind::Update)
        .unwrap();
    assert!(
        update.detail.contains("1 unattributed update page"),
        "{update:?}"
    );
}

fn ivar_tree(layout: &Layout) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(
        dir: &std::path::Path,
        base: &std::path::Path,
        out: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(&path, base, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let base = layout.root().join(".ivar");
    let mut out = std::collections::BTreeMap::new();
    walk(base.as_std_path(), base.as_std_path(), &mut out);
    out
}

#[test]
fn a_read_only_review_matches_the_review_and_writes_nothing() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    captured_approval(&layout, "2099-01-01T00:00:00Z");
    let name = FeatureName::new("checkout").unwrap();
    let normal = review_in(&layout, &name).unwrap().value;
    let before = ivar_tree(&layout);

    let read_only = review_read_only_in(&layout, &name).unwrap().value;

    assert_eq!(ivar_tree(&layout), before);
    assert_eq!(
        serde_json::to_value(&read_only.written).unwrap(),
        serde_json::to_value(&normal.written).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&read_only.gaps).unwrap(),
        serde_json::to_value(&normal.gaps).unwrap()
    );
    assert_eq!(read_only.stale.len(), normal.stale.len());
    assert_eq!(read_only.last, normal.last);
    assert!(
        read_only
            .warnings
            .iter()
            .any(|warning| warning.contains("not drained")),
        "{:?}",
        read_only.warnings
    );
}

#[test]
fn a_read_only_review_leaves_a_pending_spool_undrained_and_lists_it() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    review_in(&layout, &FeatureName::new("checkout").unwrap()).unwrap();
    let spool = layout.memory_spool_dir();
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(spool.join("pending.jsonl"), "{}\n").unwrap();
    let before = ivar_tree(&layout);

    let report = review_read_only_in(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .value;

    assert_eq!(ivar_tree(&layout), before);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("1 file(s), 3 bytes")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn a_read_only_review_refuses_an_outdated_schema_without_migrating_it() {
    let (_guard, layout) = hall_with_approved_analysis();
    session_that_wrote_an_update(&layout);
    review_in(&layout, &FeatureName::new("checkout").unwrap()).unwrap();
    rusqlite::Connection::open(layout.memory_db().as_std_path())
        .unwrap()
        .execute_batch("UPDATE memory_schema SET version = 5;")
        .unwrap();
    let before = ivar_tree(&layout);

    let failure = review_read_only_in(&layout, &FeatureName::new("checkout").unwrap()).unwrap_err();

    assert!(
        failure.what.contains("ivar memory status"),
        "{}",
        failure.what
    );
    assert_eq!(ivar_tree(&layout), before);
}
