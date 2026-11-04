#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::fixture::*;
use super::*;
use crate::action::feature::deliver::outcome::LandResult;
use crate::domain::delivery_event::DeliveryEvent;
use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::test_support::RecordingSink;
use std::sync::Arc;

fn fingerprint_of(root: &Utf8PathBuf) -> String {
    preview_json(root, preview_input("checkout"))["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn sources(sink: &RecordingSink) -> Vec<&'static str> {
    sink.records()
        .iter()
        .map(|r| r.event.source_event().as_str())
        .collect()
}

#[test]
fn a_pull_request_delivery_emits_checks_pushed_and_pr_under_one_attempt() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());
    let fingerprint = fingerprint_of(&root);

    deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap();

    assert_eq!(
        sources(&sink),
        vec!["delivery.checks", "delivery.pushed", "delivery.pr"]
    );
    let records = sink.records();
    assert!(
        records
            .iter()
            .all(|r| r.attempt_id == records[0].attempt_id)
    );
    assert!(
        records
            .iter()
            .all(|r| r.scope.repo == "api" && r.scope.feature == "checkout")
    );
    assert!(!records[1].scope.head_sha.is_empty());
    match &records[2].event {
        DeliveryEvent::Pr {
            action,
            number,
            draft,
            ..
        } => {
            assert_eq!(action, "new");
            assert_eq!(*number, 1);
            assert!(!draft);
        }
        other => panic!("expected pr, got {other:?}"),
    }
}

#[test]
fn a_preview_emits_nothing() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    deliver(&ctx, preview_input("checkout")).unwrap();

    assert!(sink.records().is_empty());
}

#[test]
fn a_refused_apply_emits_one_refused_event_with_the_presented_fingerprint() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = deliver(&ctx, apply_input("checkout", "stale-fingerprint")).unwrap_err();

    assert_eq!(failure.code, "deliver.fingerprint_mismatch");
    let records = sink.records();
    assert_eq!(records.len(), 1);
    match &records[0].event {
        DeliveryEvent::Refused {
            code, fingerprint, ..
        } => {
            assert_eq!(code, "deliver.fingerprint_mismatch");
            assert_eq!(fingerprint, "stale-fingerprint");
        }
        other => panic!("expected refused, got {other:?}"),
    }
}

#[test]
fn a_failed_land_check_records_the_failing_repo_before_the_refusal() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    approve_through_plan(&root);
    let layout = Layout::at(&root);
    let manifest = crate::action::read_manifest(&layout).unwrap();
    let repos: Vec<_> = manifest
        .repos()
        .iter()
        .map(|r| r.clone().with_checks(vec!["exit 1".to_owned()]))
        .collect();
    Manifest::write(
        &layout,
        &Manifest::new(manifest.name().clone(), manifest.providers().clone(), repos).unwrap(),
    )
    .unwrap();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());
    let preview = deliver(&ctx, land_preview_input("checkout"))
        .unwrap()
        .value
        .preview;

    let failure = deliver(
        &ctx,
        DeliverInput {
            preview: false,
            land: true,
            fingerprint: Some(preview.fingerprint),
            ..land_preview_input("checkout")
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "deliver.checks_failed");
    assert_eq!(sources(&sink), vec!["delivery.checks", "delivery.refused"]);
    assert_eq!(sink.records()[0].event.importance().get(), 7);
    assert_eq!(sink.records()[0].attempt_id, sink.records()[1].attempt_id);
}

#[test]
fn redelivering_an_open_pull_request_emits_pr_with_the_update_action() {
    let (_guard, root, fake) = on_github(&["api"]);
    let _gh = fake_gh_on_this_thread(&fake);
    deliver_json(&root, preview_input("checkout"));
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root.clone()).with_delivery_events(sink.clone());
    let retitled = metadata_input(Some("feat: updated title"), None, None);
    let fingerprint = preview_json(&root, retitled.clone())["preview"]["fingerprint"]
        .as_str()
        .unwrap()
        .to_owned();

    deliver(
        &ctx,
        DeliverInput {
            preview: false,
            fingerprint: Some(fingerprint),
            ..retitled
        },
    )
    .unwrap();

    let records = sink.records();
    let pr = records
        .iter()
        .find(|r| r.event.source_event().as_str() == "delivery.pr")
        .expect("a pr event");
    assert!(matches!(&pr.event, DeliveryEvent::Pr { action, .. } if action == "update"));
}

#[test]
fn a_land_result_maps_to_landed_and_a_failed_push_is_a_failure() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let layout = Layout::at(&root);
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());
    let land = [
        LandResult {
            repo: RepoName::new("api").unwrap(),
            merged: true,
            pushed: true,
            detail: None,
        },
        LandResult {
            repo: RepoName::new("api").unwrap(),
            merged: true,
            pushed: false,
            detail: Some("rejected".into()),
        },
    ];

    crate::action::feature::deliver::events::emit_land_outcome(
        &ctx.emitter(&layout),
        &git::System,
        &layout,
        &feature,
        &land,
    );

    let records = sink.records();
    assert_eq!(
        records
            .iter()
            .map(|r| r.event.importance().get())
            .collect::<Vec<_>>(),
        vec![4, 7]
    );
    assert!(
        matches!(&records[1].event, DeliveryEvent::Landed { merged: true, pushed: false, detail: Some(d) } if d == "rejected")
    );
}
