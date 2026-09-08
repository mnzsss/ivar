#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::name::SessionId;
use crate::test_support::hall_root;

#[test]
fn writeset_new_and_properties() {
    let session = SessionId::new("00000000-0000-0000-0000-000000000001").unwrap();
    let empty = MemoryWriteSet::new(session.clone(), Vec::new());
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);

    let set = MemoryWriteSet::new(
        session.clone(),
        vec![
            Utf8PathBuf::from("memory/topics/auth.md"),
            Utf8PathBuf::from("memory/episodes/ep1.md"),
        ],
    );
    assert!(!set.is_empty());
    assert_eq!(set.len(), 2);
    assert_eq!(set.session, session);
}

#[test]
fn writeset_serialization_roundtrip() {
    let session = SessionId::new("00000000-0000-0000-0000-000000000002").unwrap();
    let set = MemoryWriteSet::new(
        session,
        vec![
            Utf8PathBuf::from("memory/topics/arch.md"),
            Utf8PathBuf::from("memory/index.sqlite"),
        ],
    );

    let json = set.to_json().expect("serialize to json");
    let deserialized = MemoryWriteSet::from_json(&json).expect("deserialize from json");
    assert_eq!(set, deserialized);
}

#[test]
fn writeset_filter_existing() {
    let (_guard, root) = hall_root();
    let session = SessionId::new("00000000-0000-0000-0000-000000000003").unwrap();

    let existing_rel = Utf8PathBuf::from("memory/topics/exists.md");
    let missing_rel = Utf8PathBuf::from("memory/topics/missing.md");

    let full_existing = root.join(&existing_rel);
    crate::infra::fs::ensure_dir(full_existing.parent().unwrap()).unwrap();
    crate::infra::fs::write_atomic(&full_existing, b"exists").unwrap();

    let set = MemoryWriteSet::new(session, vec![existing_rel.clone(), missing_rel]);

    let filtered = set.filter_existing(&root);
    assert_eq!(filtered.modified_paths, vec![existing_rel]);
}
