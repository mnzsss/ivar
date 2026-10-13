#![allow(clippy::unwrap_used)]

use crate::action::graph::outcome::{IndexBatchOutcome, McpRegistration};
use crate::action::graph::reconcile::{Removed, RemovedLayer};
use crate::error::WriteHuman;

fn outcome(removed: Removed) -> IndexBatchOutcome {
    IndexBatchOutcome {
        repos: vec![],
        repos_failed: vec![],
        cross_edges_linked: 0,
        mcp_registration: McpRegistration::Skipped,
        next_command: None,
        removed,
    }
}

#[test]
fn the_human_surface_names_every_removed_repo_layer_and_scope() {
    let out = outcome(Removed {
        repos: vec!["orca".into()],
        layers: vec![RemovedLayer {
            feature: "old".into(),
            repo: "api".into(),
        }],
        scopes: vec!["base:ivar-bb".into()],
        hall_files: 2,
        skipped: None,
    });
    let mut buf = Vec::new();
    out.write_human(&mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();

    assert!(text.contains("Removed stale graph state:"), "{text}");
    assert!(text.contains("repo orca"), "{text}");
    assert!(text.contains("layer old/api"), "{text}");
    assert!(text.contains("scope base:ivar-bb"), "{text}");
    assert!(text.contains("2 hall files outside graph.paths"), "{text}");
}

#[test]
fn nothing_removed_prints_nothing_extra_and_serialises_no_removed_key() {
    let out = outcome(Removed::default());
    let mut buf = Vec::new();
    out.write_human(&mut buf).unwrap();
    assert!(!String::from_utf8(buf).unwrap().contains("Removed"));

    let json = serde_json::to_value(&out).unwrap();
    assert!(json.get("removed").is_none(), "{json}");
}
