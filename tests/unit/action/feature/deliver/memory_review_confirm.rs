use super::fixture::*;
use super::memory_review::{checkout, scope};
use super::*;
use crate::action::memory::events::record_memory_event;
use crate::action::memory::review::{ReviewState, confirm_in, review_state};
use crate::domain::observation::SourceEvent;
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
