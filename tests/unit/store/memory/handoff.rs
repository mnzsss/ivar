use crate::domain::memory::handoff::HandoffPayload;
use crate::domain::name::{FeatureName, SessionId};
use crate::store::layout::Layout;
use crate::store::memory::handoff::{claim_pending_handoffs, persist_handoff};
use crate::test_support::utf8_temp_dir;

#[test]
fn test_persist_and_claim_handoffs_lifecycle() {
    let (_guard, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let feature = FeatureName::new("feat-alpha").unwrap();

    let handoff1 = HandoffPayload::new(
        "handoff-01",
        SessionId::new("11111111-1111-1111-1111-111111111111").unwrap(),
        "Finished step 1",
        vec!["Task A".to_string()],
        vec!["Decision X".to_string()],
        vec!["src/a.rs".to_string()],
    );

    let handoff2 = HandoffPayload::new(
        "handoff-02",
        SessionId::new("22222222-2222-2222-2222-222222222222").unwrap(),
        "Finished step 2",
        vec!["Task B".to_string()],
        vec!["Decision Y".to_string()],
        vec!["src/b.rs".to_string()],
    );

    persist_handoff(&layout, &feature, &handoff2).unwrap();
    persist_handoff(&layout, &feature, &handoff1).unwrap();

    assert!(layout.feature_memory_inbox(&feature).join("handoff-01.json").exists());
    assert!(layout.feature_memory_inbox(&feature).join("handoff-02.json").exists());

    let claimed = claim_pending_handoffs(&layout, &feature).unwrap();
    assert_eq!(claimed.len(), 2);
    assert_eq!(claimed[0].id, "handoff-01");
    assert_eq!(claimed[1].id, "handoff-02");

    // Inbox is now empty
    assert!(!layout.feature_memory_inbox(&feature).join("handoff-01.json").exists());
    assert!(!layout.feature_memory_inbox(&feature).join("handoff-02.json").exists());

    // Archive has files
    assert!(layout.feature_memory_archive(&feature).join("handoff-01.json").exists());
    assert!(layout.feature_memory_archive(&feature).join("handoff-02.json").exists());

    // Second claim produces empty list
    let claimed_again = claim_pending_handoffs(&layout, &feature).unwrap();
    assert!(claimed_again.is_empty());
}
