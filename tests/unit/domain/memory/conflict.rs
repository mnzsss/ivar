#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::domain::memory::ScopeName;
use crate::domain::memory::conflict::{list_pending_conflicts, preserve_topic_conflict};
use crate::infra::fs;
use crate::store::layout::Layout;

#[test]
fn test_preserve_topic_conflict_creates_file_when_not_exists() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let layout = Layout::at(camino::Utf8Path::from_path(tmp.path()).expect("utf8"));
    let scope = ScopeName::new("engineering").unwrap();

    let res = preserve_topic_conflict(
        &layout,
        &scope,
        "auth-pattern",
        "# Auth Pattern\n\nInitial content.",
    )
    .expect("preserve");

    let topic_path = layout.memory_topic(&scope, "auth-pattern");
    assert_eq!(res.preserved_files, vec![topic_path.clone()]);
    assert_eq!(
        fs::read_text(&topic_path).unwrap().unwrap(),
        "# Auth Pattern\n\nInitial content."
    );
}

#[test]
fn test_preserve_topic_conflict_identical_content_no_conflict() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let layout = Layout::at(camino::Utf8Path::from_path(tmp.path()).expect("utf8"));
    let scope = ScopeName::new("engineering").unwrap();
    let content = "# Auth Pattern\n\nInitial content.";
    let res1 =
        preserve_topic_conflict(&layout, &scope, "auth-pattern", content).expect("preserve 1");
    assert!(!res1.requires_user_action);

    let res2 =
        preserve_topic_conflict(&layout, &scope, "auth-pattern", content).expect("preserve 2");
    let topic_path = layout.memory_topic(&scope, "auth-pattern");
    assert_eq!(res2.preserved_files, vec![topic_path]);
    assert!(!res2.requires_user_action);
}

#[test]
fn test_preserve_topic_conflict_preserves_both_when_differing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let layout = Layout::at(camino::Utf8Path::from_path(tmp.path()).expect("utf8"));
    let scope = ScopeName::new("engineering").unwrap();

    let original = "# Auth Pattern\n\nOriginal content.";
    let incoming = "# Auth Pattern\n\nConflicting incoming content.";

    preserve_topic_conflict(&layout, &scope, "auth-pattern", original).expect("orig");
    let topic_path = layout.memory_topic(&scope, "auth-pattern");
    let res = preserve_topic_conflict(&layout, &scope, "auth-pattern", incoming).expect("conflict");

    assert!(res.requires_user_action);
    assert_eq!(res.preserved_files.len(), 2);
    assert_eq!(res.preserved_files[0], topic_path);
    assert!(
        res.preserved_files[1]
            .as_str()
            .contains("auth-pattern.conflict-")
    );
    assert!(res.preserved_files[1].as_str().ends_with(".md"));

    // Verify disk content
    assert_eq!(fs::read_text(&topic_path).unwrap().unwrap(), original);
    assert_eq!(
        fs::read_text(&res.preserved_files[1]).unwrap().unwrap(),
        incoming
    );
}

#[test]
fn test_list_pending_conflicts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let layout = Layout::at(camino::Utf8Path::from_path(tmp.path()).expect("utf8"));
    let scope = ScopeName::new("engineering").unwrap();

    let original = "# Topic\n\nOriginal";
    let conflict = "# Topic\n\nConflict";

    preserve_topic_conflict(&layout, &scope, "topic1", original).unwrap();
    assert!(list_pending_conflicts(&layout).unwrap().is_empty());

    preserve_topic_conflict(&layout, &scope, "topic1", conflict).unwrap();
    let conflicts = list_pending_conflicts(&layout).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].as_str().contains("topic1.conflict-"));
}
