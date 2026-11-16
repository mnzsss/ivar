use super::fixture::*;
use super::*;
use crate::domain::delivery_event::DeliveryEvent;
use crate::test_support::RecordingSink;
use std::sync::Arc;

fn preview(root: &Utf8Path) -> Report<DeliverOutcome> {
    deliver(&Ctx::new(root.to_path_buf()), preview_input("checkout")).unwrap()
}

fn fingerprint(root: &Utf8Path) -> String {
    preview(root).value.preview.fingerprint
}

fn no_plan_input(fingerprint: &str, reason: &str) -> DeliverInput {
    DeliverInput {
        no_plan: Some(reason.to_owned()),
        ..apply_input("checkout", fingerprint)
    }
}

fn skipped_reasons(sink: &RecordingSink) -> Vec<String> {
    sink.records()
        .iter()
        .filter_map(|record| match &record.event {
            DeliveryEvent::PlanSkipped { reason } => Some(reason.clone()),
            _ => None,
        })
        .collect()
}

/// A hall whose `checkout` wrote no SPDD artifact but reviewed its memory.
fn hall_without_plan() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with_promoted(&["api"]);
    record_memory_review(&root, "checkout", serde_json::json!({}));
    (guard, root)
}

fn create_plan_artifacts(root: &Utf8Path) {
    crate::action::plan::create::create(
        &Ctx::new(root.to_path_buf()),
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
}

#[test]
fn the_preview_of_a_feature_without_artifacts_names_no_plan() {
    let (_guard, root) = hall_without_plan();

    let blockers = preview(&root).value.blockers;

    let plan: Vec<_> = blockers
        .iter()
        .filter(|b| b.contains("plan gate"))
        .collect();
    assert_eq!(plan.len(), 1, "{blockers:?}");
    assert!(plan[0].contains("--no-plan \"<reason>\""), "{plan:?}");
    assert!(plan[0].contains("no plan was written"), "{plan:?}");
}

#[test]
fn the_preview_of_a_written_plan_does_not_offer_no_plan() {
    let (_guard, root) = hall_without_plan();
    create_plan_artifacts(&root);

    let blockers = preview(&root).value.blockers;

    assert!(
        blockers.iter().any(|b| b.contains("plan gate")),
        "{blockers:?}"
    );
    assert!(
        blockers.iter().all(|b| !b.contains("--no-plan")),
        "{blockers:?}"
    );
}

#[test]
fn the_apply_refusal_without_a_plan_names_no_plan() {
    let (_guard, root) = hall_without_plan();
    let fp = fingerprint(&root);

    let failure = deliver(&Ctx::new(root), apply_input("checkout", &fp)).unwrap_err();

    assert_eq!(failure.code, "deliver.plan_not_approved");
    let text = failure_text(&failure);
    assert!(text.contains("--no-plan"), "{text}");
}

#[test]
fn no_plan_lifts_the_plan_gate_and_records_the_reason() {
    let (_guard, root) = hall_without_plan();
    let fp = fingerprint(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let report = deliver(&ctx, no_plan_input(&fp, "typo fix")).unwrap();

    assert!(!report.value.pushes.is_empty(), "{:?}", report.value);
    assert_eq!(skipped_reasons(&sink), ["typo fix"]);
    let records = sink.records();
    let skipped = records
        .iter()
        .find(|r| matches!(r.event, DeliveryEvent::PlanSkipped { .. }))
        .unwrap();
    assert_eq!(skipped.scope.feature, "checkout");
    assert_eq!(
        skipped.event.source_event().as_str(),
        "delivery.plan_skipped"
    );
}

#[test]
fn no_plan_is_not_part_of_the_fingerprint() {
    let (_guard, root) = hall_without_plan();
    let before = fingerprint(&root);

    let applied = deliver(&Ctx::new(root), no_plan_input(&before, "hotfix"));

    assert!(applied.is_ok(), "{applied:?}");
}

#[test]
fn no_plan_with_the_preview_is_refused() {
    let (_guard, root) = hall_without_plan();

    let failure = deliver(
        &Ctx::new(root),
        DeliverInput {
            no_plan: Some("hotfix".to_owned()),
            ..preview_input("checkout")
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "deliver.no_plan_with_preview");
}

#[test]
fn no_plan_with_a_blank_reason_is_refused() {
    let (_guard, root) = hall_without_plan();
    let fp = fingerprint(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = deliver(&ctx, no_plan_input(&fp, "   ")).unwrap_err();

    assert_eq!(failure.code, "deliver.no_plan_reason_required");
    assert!(skipped_reasons(&sink).is_empty());
}

#[test]
fn no_plan_still_refuses_a_dirty_repo_and_records_nothing() {
    let (_guard, root) = hall_without_plan();
    std::fs::write(root.join(".ivar/repos/api/checkout/wip.md"), "wip\n").unwrap();
    let fp = fingerprint(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = deliver(&ctx, no_plan_input(&fp, "hotfix")).unwrap_err();

    assert_eq!(failure.code, "deliver.blocked");
    let actual = failure.actual.clone().unwrap_or_default();
    assert!(actual.contains("uncommitted"), "{actual}");
    assert!(!actual.contains("plan gate"), "{actual}");
    assert!(skipped_reasons(&sink).is_empty());
}

#[test]
fn no_plan_still_refuses_a_missing_memory_review_and_records_nothing() {
    let (_guard, root) = hall_with_promoted(&["api"]);
    let fp = fingerprint(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = deliver(&ctx, no_plan_input(&fp, "hotfix")).unwrap_err();

    assert_eq!(failure.code, "deliver.memory_not_reviewed");
    assert!(skipped_reasons(&sink).is_empty());
}

#[test]
fn no_plan_is_refused_when_a_plan_exists_and_points_at_approval() {
    let (_guard, root) = hall_without_plan();
    create_plan_artifacts(&root);
    let fp = fingerprint(&root);
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = deliver(&ctx, no_plan_input(&fp, "hotfix")).unwrap_err();

    assert_eq!(failure.code, "deliver.no_plan_with_artifacts");
    let text = failure_text(&failure);
    assert!(text.contains("ivar plan approve checkout plan"), "{text}");
    assert!(skipped_reasons(&sink).is_empty());
}

#[test]
fn no_plan_is_refused_even_when_the_plan_is_approved() {
    let (_guard, root) = hall_without_plan();
    approve_plan_gates(&root);
    let fp = fingerprint(&root);

    let failure = deliver(&Ctx::new(root), no_plan_input(&fp, "hotfix")).unwrap_err();

    assert_eq!(failure.code, "deliver.no_plan_with_artifacts");
}

#[test]
fn a_plan_written_after_the_preview_voids_no_plan() {
    let (_guard, root) = hall_without_plan();
    let fp = fingerprint(&root);
    // Only the requirements artifact: the plan gate stays pending, so the
    // fingerprint may drift, but the artifact check never depends on it.
    let layout = Layout::at(root.clone());
    let dir = layout.plan_dir(&FeatureName::new("checkout").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("requirements.md"), "# requirements\n").unwrap();
    let sink = Arc::new(RecordingSink::default());
    let ctx = Ctx::new(root).with_delivery_events(sink.clone());

    let failure = deliver(&ctx, no_plan_input(&fp, "hotfix")).unwrap_err();

    assert_eq!(failure.code, "deliver.no_plan_with_artifacts");
    assert!(skipped_reasons(&sink).is_empty());
}

#[test]
fn land_follows_the_same_rule() {
    let (_guard, root) = hall_without_plan();
    let ctx = Ctx::new(root.clone());
    let fp = deliver(&ctx, land_preview_input("checkout"))
        .unwrap()
        .value
        .preview
        .fingerprint;
    let land = |no_plan: Option<&str>| DeliverInput {
        preview: false,
        fingerprint: Some(fp.clone()),
        no_plan: no_plan.map(str::to_owned),
        ..land_preview_input("checkout")
    };

    let refused = deliver(&ctx, land(None)).unwrap_err();
    assert_eq!(refused.code, "deliver.plan_not_approved");

    let sink = Arc::new(RecordingSink::default());
    let ctx = ctx.with_delivery_events(sink.clone());
    let applied = deliver(&ctx, land(Some("hotfix")));
    if let Err(failure) = &applied {
        assert_ne!(failure.code, "deliver.plan_not_approved", "{failure:?}");
        assert_ne!(
            failure.code, "deliver.no_plan_with_artifacts",
            "{failure:?}"
        );
    }
    assert_eq!(skipped_reasons(&sink), ["hotfix"]);
}
