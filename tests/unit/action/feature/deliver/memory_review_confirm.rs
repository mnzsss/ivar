use super::fixture::*;
use super::memory_review::{checkout, scope, session_tool};
use super::*;
use crate::action::memory::events::record_memory_event;
use crate::action::memory::review::{ReviewState, confirm_in, review_in, review_state};
use crate::domain::memory_review::GapKind;
use crate::domain::observation::{SourceEvent, ToolEffect};
use serde_json::json;

fn page_written(layout: &Layout, path: &str) {
    record_memory_event(
        layout,
        &scope(),
        SourceEvent::MemoryPageWritten,
        &json!({"path": path, "kind": "gotcha", "action": "written", "sha256": "h"}),
    )
    .unwrap();
}

#[test]
fn not_required_is_refused_once_the_feature_wrote_a_page() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    page_written(&layout, "docs/memory/gotcha/g.md");

    let refused = confirm_in(
        &layout,
        &scope(),
        &checkout(),
        &[],
        Some("internal refactor".to_owned()),
    )
    .unwrap_err();

    assert_eq!(refused.code, "memory.review_not_required_with_pages");
    assert!(refused.what.contains("wrote 1"), "{}", refused.what);
    assert_eq!(
        review_state(&layout, &checkout()).unwrap(),
        ReviewState::Missing
    );
}

#[test]
fn a_confirmation_records_the_pages_the_feature_wrote() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root);
    page_written(&layout, "docs/memory/gotcha/g.md");

    let report = confirm_in(&layout, &scope(), &checkout(), &[], None)
        .unwrap()
        .value;

    assert_eq!(report.last.unwrap().written, ["docs/memory/gotcha/g.md"]);
}

#[test]
fn a_page_written_in_a_descendant_session_counts_for_the_root_review() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(root.clone());
    child_of_checkout(&root, "checkout-ui");
    session_tool(
        &layout,
        "child-write",
        "checkout-ui",
        "2026-11-05T10:00:00Z",
        ToolEffect::Writes,
        json!({"tool": "Write", "input": {"file_path": format!("{root}/docs/memory/update/u.md")}, "result": ""}),
    );

    let report = review_in(&layout, &checkout()).unwrap().value;

    let written: Vec<&str> = report.written.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(written, ["docs/memory/update/u.md"]);
    assert!(
        !report.gaps.iter().any(|g| g.kind == GapKind::Update),
        "{:?}",
        report.gaps
    );
}
