use crate::domain::memory::episode::EpisodePayload;
use crate::domain::name::{FeatureName, SessionId};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::memory::episode::persist_episode;
use crate::test_support::utf8_temp_dir;

#[test]
fn test_persist_episode() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    let session_id = SessionId::new("11111111-1111-1111-1111-111111111111").unwrap();
    let episode = EpisodePayload::new(
        session_id.clone(),
        Some(FeatureName::new("shared-memory").unwrap()),
        "2026-09-08T10:00:00Z",
        "2026-09-08T11:00:00Z",
        "Completed task 6",
        vec!["src/lib.rs".to_string()],
    );

    persist_episode(&layout, &episode).unwrap();

    let episode_path = layout.memory_episodes_dir().join("11111111-1111-1111-1111-111111111111.md");
    assert!(episode_path.exists());

    let content = fs::read_text(&episode_path).unwrap().unwrap();
    assert!(content.contains("# Session Episode: 11111111-1111-1111-1111-111111111111"));
    assert!(content.contains("- **Feature**: `shared-memory`"));
    assert!(content.contains("Completed task 6"));
    assert!(content.contains("- `src/lib.rs`"));
}
