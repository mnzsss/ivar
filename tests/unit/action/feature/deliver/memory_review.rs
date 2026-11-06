use super::fixture::*;
use super::*;
use crate::action::memory::events::{MemoryEventScope, record_memory_event};
use crate::action::memory::review::{ReviewState, confirm_in, review_in, review_state};
use crate::domain::memory_review::{GapKind, StaleReason, Verdict};
use crate::domain::name::FeatureName;
use crate::domain::observation::{Importance, Observation, SourceEvent, ToolEffect};
use crate::domain::sanitize::{SanitizeContext, Sanitized};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::observations::insert_batch;
use serde_json::json;

pub(super) fn checkout() -> FeatureName {
    FeatureName::new("checkout").unwrap()
}

pub(super) fn scope() -> MemoryEventScope {
    MemoryEventScope {
        session_id: "s1".to_owned(),
        feature: "checkout".to_owned(),
        parent_feature: String::new(),
    }
}

fn approve_analysis(root: &Utf8PathBuf) {
    let ctx = Ctx::new(root.clone());
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
}

pub(super) fn session_tool(
    layout: &Layout,
    id: &str,
    feature: &str,
    at: &str,
    effect: ToolEffect,
    body: serde_json::Value,
) {
    let observation = Observation {
        event_id: id.to_owned(),
        source_event: "session.tool".to_owned(),
        occurred_at: at.to_owned(),
        session_id: "s1".to_owned(),
        native_session_id: "n1".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0".to_owned(),
        feature: feature.to_owned(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::session_tool(effect),
        attempt_id: String::new(),
        body: body.to_string(),
    };
    let ctx = SanitizeContext::new(layout.root().to_path_buf(), Vec::new());
    let mut db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    insert_batch(&mut db, &[Sanitized::sanitize(observation, &ctx).unwrap()]).unwrap();
}

fn anchored_page(layout: &Layout, page: &str, repo: &str, path: &str) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .execute(
            "INSERT INTO pages (path, kind, tier, title, size, mtime_ns, content_hash, indexed_at)
             VALUES (?1, 'decision', 'semantic', 'A', 1, 1, 'h', 't')",
            [page],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO page_anchors (page_path, repo, path, symbol) VALUES (?1, ?2, ?3, NULL)",
            [page, repo, path],
        )
        .unwrap();
}

#[test]
fn the_review_lists_written_pages_gaps_and_pages_anchored_to_changed_files() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root.clone());
    approve_analysis(&root);
    std::fs::write(
        layout
            .repo_worktree(
                &RepoName::new("api").unwrap(),
                &BranchName::new("checkout").unwrap(),
            )
            .join("README.md"),
        "changed\n",
    )
    .unwrap();
    anchored_page(&layout, "docs/memory/decision/a.md", "api", "README.md");
    session_tool(
        &layout,
        "fail",
        "",
        "2026-11-05T10:00:00Z",
        ToolEffect::Failed,
        json!({"tool": "Bash", "input": {"command": "cargo test"}, "result": "1 failed"}),
    );
    session_tool(
        &layout,
        "query",
        "checkout",
        "2026-11-05T10:01:00Z",
        ToolEffect::Reads,
        json!({"tool": "mcp__acme-ivar__memory_query", "path": ""}),
    );
    record_memory_event(&layout, &scope(), SourceEvent::MemoryPageWritten, &json!({"path": "docs/memory/gotcha/g.md", "kind": "gotcha", "action": "written", "sha256": "h"})).unwrap();

    let report = review_in(&layout, &checkout()).unwrap().value;

    let written: Vec<&str> = report.written.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(written, ["docs/memory/gotcha/g.md"]);
    let gaps: Vec<GapKind> = report.gaps.iter().map(|g| g.kind).collect();
    assert_eq!(gaps, [GapKind::Update]);
    assert_eq!(report.stale.len(), 1);
    assert_eq!(report.stale[0].path, "docs/memory/decision/a.md");
    assert_eq!(
        report.stale[0].reasons,
        [StaleReason::ChangedFile {
            repo: "api".to_owned(),
            path: "README.md".to_owned()
        }]
    );
    assert_eq!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Missing
    );
}

#[test]
fn confirming_needs_a_verdict_per_stale_page_then_records_the_heads() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root.clone());
    let api = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    std::fs::write(api.join("README.md"), "changed\n").unwrap();
    anchored_page(&layout, "docs/memory/decision/a.md", "api", "README.md");

    let refused = confirm_in(&layout, &scope(), &checkout(), &[], None).unwrap_err();
    assert_eq!(refused.code, "memory.review_missing_verdicts");
    assert!(
        refused.what.contains("docs/memory/decision/a.md"),
        "{}",
        refused.what
    );

    let report = confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[("docs/memory/decision/a.md".to_owned(), Verdict::StillValid)],
        None,
    )
    .unwrap()
    .value;

    let head = git_stdout(&api, &["rev-parse", "HEAD"]).trim().to_owned();
    let last = report.last.unwrap();
    assert_eq!(last.source_event, "memory.reviewed");
    assert_eq!(last.stale, ["docs/memory/decision/a.md"]);
    assert!(last.written.is_empty(), "{last:?}");
    assert_eq!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Reviewed {
            heads: [("api".to_owned(), head)].into_iter().collect()
        }
    );
}

#[test]
fn a_skip_alone_is_skipped_and_a_review_outranks_a_later_skip() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    record_memory_event(
        &layout,
        &scope(),
        SourceEvent::MemoryReviewSkipped,
        &json!({"reason": "hotfix"}),
    )
    .unwrap();
    crate::action::memory::drain::drain(&layout).unwrap();
    assert_eq!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Skipped
    );

    confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal refactor".to_owned()),
    )
    .unwrap();
    record_memory_event(
        &layout,
        &scope(),
        SourceEvent::MemoryReviewSkipped,
        &json!({"reason": "again"}),
    )
    .unwrap();
    crate::action::memory::drain::drain(&layout).unwrap();
    assert!(matches!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Reviewed { .. }
    ));
}

#[test]
fn without_a_memory_database_the_review_is_empty_with_a_warning_and_the_state_is_missing() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    let _ = std::fs::remove_file(layout.memory_db());

    assert_eq!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Missing
    );
    let report = review_in(&layout, &checkout()).unwrap().value;
    assert!(report.written.is_empty() && report.stale.is_empty());
    assert_eq!(
        report.gaps.iter().map(|g| g.kind).collect::<Vec<_>>(),
        [GapKind::Update]
    );
}

#[test]
fn an_unreadable_memory_database_is_a_warning_never_an_error() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    std::fs::write(layout.memory_db(), "not a sqlite database").unwrap();

    let report = review_in(&layout, &checkout()).unwrap().value;

    assert!(report.written.is_empty(), "{report:?}");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("memory could not be read")),
        "{:?}",
        report.warnings
    );
    let refused = confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal".to_owned()),
    )
    .unwrap_err();
    assert_eq!(refused.code, "memory.review_unreadable");
}

#[test]
fn confirming_through_the_tool_records_the_review_for_the_callers_feature() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    let caller = crate::action::memory::mcp::MemoryCaller {
        session: Some("s1".to_owned()),
        feature: Some(checkout()),
        parent_feature: None,
    };

    let text = crate::action::memory::mcp::call_as(
        &layout,
        "memory_review",
        &json!({"confirm": true, "not_required": "internal refactor"}),
        &caller,
    )
    .unwrap();

    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["feature"], "checkout");
    assert_eq!(report["last"]["source_event"], "memory.reviewed");
    assert_eq!(report["last"]["not_required"], "internal refactor");
    assert!(matches!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Reviewed { .. }
    ));
}

#[test]
fn the_human_review_names_each_list_and_the_last_confirmation() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root.clone());
    confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal refactor".to_owned()),
    )
    .unwrap();

    let report =
        crate::action::memory::review::review(&Ctx::new(root), Some("checkout".to_owned()))
            .unwrap()
            .value;
    let mut out = Vec::new();
    report.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(text.starts_with("memory review of `checkout`\n"), "{text}");
    assert!(text.contains("written: none"), "{text}");
    assert!(
        text.contains("gap update: no update page was written in `checkout`"),
        "{text}"
    );
    assert!(text.contains("stale: none"), "{text}");
    assert!(text.contains("last: memory.reviewed at "), "{text}");
}

#[test]
fn a_diff_that_cannot_be_read_is_a_warning_and_refuses_the_confirmation() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    std::fs::remove_dir_all(layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    ))
    .unwrap();

    let report = review_in(&layout, &checkout()).unwrap().value;

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("the diff of `api` could not be read")),
        "{:?}",
        report.warnings
    );
    let refused = confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal".to_owned()),
    )
    .unwrap_err();
    assert_eq!(refused.code, "memory.review_unreadable");
}

#[test]
fn every_broken_anchor_of_a_durable_page_is_stale_wherever_its_file_is() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    anchored_page(&layout, "docs/memory/decision/a.md", "api", "src/gone.rs");
    anchored_page(&layout, "docs/memory/session/s.md", "api", "src/gone.rs");
    rusqlite::Connection::open(layout.graph_db().as_std_path())
        .unwrap()
        .execute_batch(
            "CREATE TABLE files (id INTEGER PRIMARY KEY, repo TEXT, path TEXT);
             CREATE TABLE symbols (file_id INTEGER, name TEXT);",
        )
        .unwrap();

    let report = review_in(&layout, &checkout()).unwrap().value;

    let stale: Vec<&str> = report.stale.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        stale,
        ["docs/memory/decision/a.md"],
        "{:?}",
        report.warnings
    );
    assert_eq!(
        report.stale[0].reasons,
        [StaleReason::BrokenAnchor {
            repo: "api".to_owned(),
            path: "src/gone.rs".to_owned(),
            symbol: None
        }]
    );
}

#[test]
fn a_missing_graph_is_a_warning_that_never_blocks_the_confirmation() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    let _ = std::fs::remove_file(layout.graph_db());

    let report = review_in(&layout, &checkout()).unwrap().value;

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("graph.db is missing")),
        "{:?}",
        report.warnings
    );
    confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal".to_owned()),
    )
    .unwrap();
}

#[test]
fn an_unreadable_graph_is_a_warning_and_refuses_the_confirmation() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    std::fs::write(layout.graph_db(), "not a sqlite database").unwrap();

    let report = review_in(&layout, &checkout()).unwrap().value;

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("graph.db could not be read")),
        "{:?}",
        report.warnings
    );
    let refused = confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal".to_owned()),
    )
    .unwrap_err();
    assert_eq!(refused.code, "memory.review_unreadable");
}

#[test]
fn the_heads_survive_a_review_body_cut_at_the_sanitize_limit() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    let written: Vec<String> = (0..50)
        .map(|n| format!("docs/memory/decision/a-long-and-realistic-decision-page-about-the-memory-review-gate-{n}.md"))
        .collect();
    record_memory_event(
        &layout,
        &scope(),
        SourceEvent::MemoryReviewed,
        &json!({"heads": {"api": "abc123"}, "gaps": ["update"], "written": written}),
    )
    .unwrap();
    crate::action::memory::drain::drain(&layout).unwrap();

    assert_eq!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Reviewed {
            heads: [("api".to_owned(), "abc123".to_owned())]
                .into_iter()
                .collect()
        }
    );
}
