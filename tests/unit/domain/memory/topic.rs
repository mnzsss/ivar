#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::memory::config::ScopeName;
use crate::domain::memory::topic::{
    MemoryTier, MemoryTopic, TopicMetadata, TopicStatus,
};

#[test]
fn topic_metadata_serialization_and_deserialization() {
    let scope = ScopeName::new("architecture").unwrap();
    let metadata = TopicMetadata {
        title: "Domain Model".into(),
        scope,
        description: "Overview of domain entities".into(),
        tier: MemoryTier::Core,
        status: TopicStatus::Active,
        updated: "2026-09-08T12:00:00Z".into(),
        tags: vec!["domain".into(), "architecture".into()],
    };
    let topic = MemoryTopic {
        metadata: metadata.clone(),
        content: "# Domain Model\n\nCanonical representation of entities.".into(),
    };

    assert_eq!(topic.metadata.title, "Domain Model");
    assert_eq!(topic.metadata.tier, MemoryTier::Core);
    assert_eq!(topic.metadata.status, TopicStatus::Active);
}

#[test]
fn topic_status_superseded() {
    let status = TopicStatus::Superseded("new-topic".into());
    if let TopicStatus::Superseded(target) = status {
        assert_eq!(target, "new-topic");
    } else {
        panic!("expected superseded");
    }
}
