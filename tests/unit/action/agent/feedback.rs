#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::agent::receipt::{AgentRunReceipt, Rating, Trigger};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;

#[test]
fn records_the_verdict_on_the_receipt() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    crate::store::agent::write_receipt(
        &layout,
        &AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Manual, "t".into()),
    )
    .unwrap();
    let updated = feedback(
        &Ctx::new(root),
        FeedbackInput {
            run_id: "r1".into(),
            rating: "down".into(),
            comment: Some("missed the flaky test".into()),
        },
    )
    .unwrap()
    .value;
    let stored = crate::store::agent::read_receipt(&layout, "nightly", "r1")
        .unwrap()
        .unwrap();
    assert_eq!(stored, updated);
    let fb = stored.feedback.unwrap();
    assert_eq!(fb.rating, Rating::Down);
    assert_eq!(fb.comment.as_deref(), Some("missed the flaky test"));
}

#[test]
fn an_unknown_run_is_not_found() {
    let (_guard, root) = seeded_hall();
    let failure = feedback(
        &Ctx::new(root),
        FeedbackInput {
            run_id: "nope".into(),
            rating: "up".into(),
            comment: None,
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "agent.run_not_found");
}
