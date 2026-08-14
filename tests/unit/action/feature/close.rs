#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::hall::{self, InitInput};
use crate::error::Status;
use crate::test_support::hall_root;

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
        },
    )
    .unwrap();
    (guard, root)
}

fn close_input(outcome: &str) -> CloseInput {
    CloseInput {
        name: "checkout".to_owned(),
        outcome: outcome.to_owned(),
    }
}

#[test]
fn close_stops_sessions_drops_execution_and_records_the_outcome() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    // A live executor session view dir, and an execution board.
    let sessions = root.join(".ivar/features/checkout/sessions/sess-1");
    fs::ensure_dir(&sessions).unwrap();
    fs::write_text(&sessions.join("state.json"), "{}").unwrap();
    fs::ensure_dir(&root.join(".ivar/features/checkout/execution")).unwrap();

    let report = close(&ctx, close_input("delivered")).unwrap();

    assert!(report.is_clean());
    assert!(!report.value.already_closed);
    assert_eq!(report.value.outcome, PromotionOutcome::Delivered);
    assert!(!fs::exists(&sessions).unwrap());
    assert!(!fs::exists(&root.join(".ivar/features/checkout/execution")).unwrap());

    // The outcome landed in plan.md's frontmatter, body preserved.
    let plan = fs::read_text(&root.join("plans/checkout/plan.md"))
        .unwrap()
        .unwrap();
    let parsed = frontmatter::parse::<PlanFrontmatter>(&plan).unwrap();
    assert_eq!(parsed.outcome.as_deref(), Some("delivered"));
    assert!(parsed.closed_at.is_some());
}

#[test]
fn close_is_idempotent_and_does_not_overwrite_the_recorded_outcome() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    close(&ctx, close_input("delivered")).unwrap();

    // A second close, with a different outcome, is a no-op.
    let report = close(&ctx, close_input("abandoned")).unwrap();

    assert!(report.value.already_closed);
    let plan = fs::read_text(&root.join("plans/checkout/plan.md"))
        .unwrap()
        .unwrap();
    let parsed = frontmatter::parse::<PlanFrontmatter>(&plan).unwrap();
    assert_eq!(
        parsed.outcome.as_deref(),
        Some("delivered"),
        "the first recorded outcome must not be overwritten"
    );
}

#[test]
fn close_rejects_an_unknown_outcome_before_touching_anything() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = close(&ctx, close_input("shipped")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.unknown_outcome");
    assert!(!fs::exists(&root.join("plans/checkout/plan.md")).unwrap());
}

#[test]
fn close_is_rejected_for_a_missing_feature() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    let failure = close(
        &ctx,
        CloseInput {
            name: "ghost".to_owned(),
            outcome: "delivered".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.not_found");
}

#[test]
fn the_human_surface_names_the_outcome_and_timestamp() {
    let outcome = CloseOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: FeatureName::new("checkout").unwrap(),
        outcome: PromotionOutcome::Delivered,
        closed_at: "2026-08-07T12:00:00.000000000Z".to_owned(),
        already_closed: false,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Closed feature `checkout` (delivered) at 2026-08-07T12:00:00.000000000Z\n"
    );
}
