use super::*;
use crate::domain::delivery_event::DeliveryEvent;
use crate::test_support::{FakeGh, RecordingSink, fake_gh_on_this_thread};
use std::sync::Arc;

#[test]
fn a_completed_cleanup_emits_one_cleaned_event_with_the_record_and_the_docs_decision() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    let outcome = apply_with_approved_record_in(&ctx, &root);

    assert!(outcome.feature_removed);
    let records = sink.records();
    let cleaned: Vec<_> = records
        .iter()
        .filter(|r| r.event.source_event().as_str() == "delivery.cleaned")
        .collect();
    assert_eq!(cleaned.len(), 1);
    assert_eq!(cleaned[0].scope.feature, "checkout");
    match &cleaned[0].event {
        DeliveryEvent::Cleaned { record_path, docs } => {
            assert_eq!(record_path, "docs/memory/update/001-checkout.cleanup.json");
            assert_eq!(docs, "written");
        }
        other => panic!("expected cleaned, got {other:?}"),
    }
}

#[test]
fn a_cleanup_preview_emits_no_cleaned_event() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    cleanup(
        &ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap();

    assert!(
        sink.records()
            .iter()
            .all(|r| r.event.source_event().as_str() != "delivery.cleaned")
    );
}

#[test]
fn merged_and_cleaned_from_one_apply_share_one_attempt() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let api = RepoName::new("api").unwrap();
    let worktree = layout.repo_worktree(&api, &BranchName::new("checkout").unwrap());
    fs::write_text(&worktree.join("work.md"), "work\n").unwrap();
    test_git(&worktree, &["add", "work.md"]);
    test_git(&worktree, &["commit", "-m", "work"]);
    let fake = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&fake);
    fake.set_existing_pr(
        &layout.repo_bare(&api),
        "checkout",
        "https://github.com/acme/api/pull/7",
        "main",
        "MERGED",
    );
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    apply_with_approved_record_in(&ctx, &root);

    let records = sink.records();
    let sources: Vec<_> = records
        .iter()
        .map(|r| r.event.source_event().as_str())
        .collect();
    assert_eq!(sources, vec!["delivery.merged", "delivery.cleaned"]);
    assert_eq!(records[0].attempt_id, records[1].attempt_id);
}

#[test]
fn every_event_from_one_apply_shares_one_attempt() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    add_subfeature(&root, "child", "checkout", true);
    close_feature(
        &Ctx::new(root.clone()),
        CloseInput {
            name: "child".to_owned(),
            outcome: "abandoned".to_owned(),
        },
    )
    .unwrap();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());

    apply_with_approved_record_in(&ctx, &root);

    let records = sink.records();
    let cleaned: Vec<_> = records
        .iter()
        .filter(|r| r.event.source_event().as_str() == "delivery.cleaned")
        .collect();
    assert_eq!(cleaned.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.attempt_id == records[0].attempt_id)
    );
}
