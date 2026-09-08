// tests/unit/domain/memory/context.rs
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;
use tempfile::tempdir;

use crate::domain::memory::config::{MemoryConfig, MemoryScope, ScopeName};
use crate::domain::memory::context::{
    MEMORY_MANAGED_END, MEMORY_MANAGED_START, project_memory_symlink, render_memory_context,
};
use crate::domain::memory::topic::{
    MemoryTier, MemoryTopic, TopicMetadata, TopicStatus,
};
use crate::domain::name::{FeatureName, HallName};
use crate::domain::provider::Provider;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers};
use crate::store::memory::document::write_topic;

fn default_providers() -> Providers {
    Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode)
}
#[test]
fn stable_hall_and_feature_blocks_remain_byte_identical_when_hot_handoff_changes() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("rules").unwrap();
    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Coding Conventions".into(),
            scope: scope.clone(),
            description: "Strict safety rules".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T00:00:00Z".into(),
            tags: vec![],
        },
        content: "Always follow KISS principle.".into(),
    };
    write_topic(&layout, &scope, "conventions", &topic).unwrap();

    let scope_def = MemoryScope {
        id: scope,
        purpose: "Rules".into(),
        budget: 5000,
        stable_topics: vec!["conventions".into()],
    };
    let memory_cfg = MemoryConfig {
        scopes: vec![scope_def],
    };

    let manifest = Manifest::new(
        HallName::new("test-hall").unwrap(),
        default_providers(),
        vec![],
        None,
    )
    .unwrap()
    .with_memory(Some(memory_cfg))
    .unwrap();

    let feature = FeatureName::new("shared-memory").unwrap();

    let ctx1 = render_memory_context(
        &layout,
        &manifest,
        Some(&feature),
        Some("First handoff note"),
    )
    .unwrap();
    let ctx2 = render_memory_context(
        &layout,
        &manifest,
        Some(&feature),
        Some("Second different handoff note"),
    )
    .unwrap();

    // Stable context bytes must remain identical
    assert_eq!(ctx1.hall_block, ctx2.hall_block);
    assert_eq!(ctx1.feature_block, ctx2.feature_block);
    assert_ne!(ctx1.hot_block, ctx2.hot_block);
}

#[test]
fn memory_context_enforces_deterministic_ordering_and_markers() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope_b = ScopeName::new("beta").unwrap();
    let topic_b = MemoryTopic {
        metadata: TopicMetadata {
            title: "Beta Topic".into(),
            scope: scope_b.clone(),
            description: "Beta desc".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T00:00:00Z".into(),
            tags: vec![],
        },
        content: "Beta body".into(),
    };
    write_topic(&layout, &scope_b, "topic-b", &topic_b).unwrap();

    let scope_a = ScopeName::new("alpha").unwrap();
    let topic_a = MemoryTopic {
        metadata: TopicMetadata {
            title: "Alpha Topic".into(),
            scope: scope_a.clone(),
            description: "Alpha desc".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T00:00:00Z".into(),
            tags: vec![],
        },
        content: "Alpha body".into(),
    };
    write_topic(&layout, &scope_a, "topic-a", &topic_a).unwrap();

    let memory_cfg = MemoryConfig {
        scopes: vec![
            MemoryScope {
                id: scope_b,
                purpose: "Beta scope".into(),
                budget: 1000,
                stable_topics: vec!["topic-b".into()],
            },
            MemoryScope {
                id: scope_a,
                purpose: "Alpha scope".into(),
                budget: 1000,
                stable_topics: vec!["topic-a".into()],
            },
        ],
    };

    let manifest = Manifest::new(
        HallName::new("test-hall").unwrap(),
        default_providers(),
        vec![],
        None,
    )
    .unwrap()
    .with_memory(Some(memory_cfg))
    .unwrap();
    let ctx = render_memory_context(&layout, &manifest, None, None).unwrap();

    assert!(ctx.hall_block.contains(MEMORY_MANAGED_START));
    assert!(ctx.hall_block.contains(MEMORY_MANAGED_END));
    // Scopes are rendered in manifest order or deterministic order
    let idx_b = ctx.hall_block.find("Beta Topic").unwrap();
    let idx_a = ctx.hall_block.find("Alpha Topic").unwrap();
    assert!(idx_b < idx_a);
}

#[test]
fn budget_overflow_reports_diagnostic_warning_in_context() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root);

    let scope = ScopeName::new("tight").unwrap();
    let topic = MemoryTopic {
        metadata: TopicMetadata {
            title: "Large Topic".into(),
            scope: scope.clone(),
            description: "Desc".into(),
            tier: MemoryTier::Core,
            status: TopicStatus::Active,
            updated: "2026-09-08T00:00:00Z".into(),
            tags: vec![],
        },
        content: "This content is longer than ten characters.".into(),
    };
    write_topic(&layout, &scope, "large", &topic).unwrap();

    let scope_def = MemoryScope {
        id: scope,
        purpose: "Small budget".into(),
        budget: 10,
        stable_topics: vec!["large".into()],
    };
    let memory_cfg = MemoryConfig {
        scopes: vec![scope_def],
    };

    let manifest = Manifest::new(
        HallName::new("test-hall").unwrap(),
        default_providers(),
        vec![],
        None,
    )
    .unwrap()
    .with_memory(Some(memory_cfg))
    .unwrap();
    let ctx = render_memory_context(&layout, &manifest, None, None).unwrap();
    assert!(ctx.hall_block.contains("exceeds declared budget") || ctx.hall_block.contains("budget overflow") || ctx.hall_block.contains("propose condensation"));
}

#[test]
fn project_memory_symlink_creates_canonical_link() {
    let dir = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());

    // Create memory root
    std::fs::create_dir_all(layout.memory_root().as_std_path()).unwrap();

    let view_dir = root.join("session_view");
    std::fs::create_dir_all(view_dir.as_std_path()).unwrap();

    project_memory_symlink(&layout, &view_dir).unwrap();

    let link_path = view_dir.join("memory");
    assert!(link_path.is_symlink());
    let target = std::fs::read_link(link_path.as_std_path()).unwrap();
    assert_eq!(target, layout.memory_root().as_std_path());

    // Idempotent call
    project_memory_symlink(&layout, &view_dir).unwrap();
    assert!(link_path.is_symlink());
}
