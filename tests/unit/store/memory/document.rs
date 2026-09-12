#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;
use tempfile::tempdir;

use crate::domain::memory::config::ScopeName;
use crate::domain::memory::topic::{MemoryTier, MemoryTopic, TopicMetadata, TopicStatus};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::document::{delete_topic, read_topic, write_topic};

#[test]
fn atomic_topic_write_and_read_with_validated_frontmatter() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("architecture").unwrap();
    let metadata = TopicMetadata {
        title: "Domain Model".into(),
        scope: scope.clone(),
        description: "Overview of domain entities".into(),
        tier: MemoryTier::Core,
        status: TopicStatus::Active,
        updated: "2026-09-08T12:00:00Z".into(),
        tags: vec!["domain".into(), "architecture".into()],
    };
    let topic = MemoryTopic {
        metadata,
        content: "# Domain Model\n\nCanonical representation of entities.".into(),
    };

    write_topic(&layout, &scope, "domain-model", &topic).expect("write failed");
    let loaded = read_topic(&layout, &scope, "domain-model").expect("read failed");
    assert_eq!(loaded.metadata.title, "Domain Model");
    assert_eq!(
        loaded.content,
        "# Domain Model\n\nCanonical representation of entities."
    );

    delete_topic(&layout, &scope, "domain-model").expect("delete failed");
    assert!(read_topic(&layout, &scope, "domain-model").is_err());
}

#[test]
fn topic_rejects_path_traversal_and_empty_slugs() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);
    let scope = ScopeName::new("architecture").unwrap();

    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Test".into(),
            scope: scope.clone(),
            description: "Test".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec![],
        },
        content: "content".into(),
    };

    assert!(write_topic(&layout, &scope, "", &topic).is_err());
    assert!(write_topic(&layout, &scope, "../traversal", &topic).is_err());
    assert!(write_topic(&layout, &scope, "sub/dir", &topic).is_err());
    assert!(write_topic(&layout, &scope, "/root", &topic).is_err());

    assert!(read_topic(&layout, &scope, "").is_err());
    assert!(read_topic(&layout, &scope, "../traversal").is_err());
    assert!(delete_topic(&layout, &scope, "../traversal").is_err());
}

#[test]
fn topic_rejects_symlinks() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(&root);
    let scope = ScopeName::new("architecture").unwrap();

    let scope_dir = layout.memory_scope_dir(&scope);
    fs::ensure_dir(&scope_dir).unwrap();

    let target = root.join("target.md");
    fs::write_text(&target, "dummy").unwrap();

    let symlink_path = layout.memory_topic(&scope, "symlinked");
    fs::create_symlink(&target, &symlink_path).unwrap();

    assert!(read_topic(&layout, &scope, "symlinked").is_err());

    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Test".into(),
            scope: scope.clone(),
            description: "Test".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T12:00:00Z".into(),
            tags: vec![],
        },
        content: "content".into(),
    };
    assert!(write_topic(&layout, &scope, "symlinked", &topic).is_err());
    assert!(delete_topic(&layout, &scope, "symlinked").is_err());
}
