//! Text checks over the phase transitions and the `ivar-plan` templates.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::shipped::shipped;

#[test]
fn phase_transitions_offer_approve_and_continue_and_templates_match_execute() {
    for path in ["skills/ivar-plan/SKILL.md", "skills/ivar-execute/SKILL.md"] {
        assert!(
            shipped(path).contains("Approve and continue"),
            "{path} needs a combined gate"
        );
    }
    let template = shipped("skills/ivar-plan/references/plan-template.md");
    for stale in [
        "Deferred validation failures",
        "Human approval",
        "✅",
        "| Done |",
    ] {
        assert!(
            !template.contains(stale),
            "plan-template still tracks progress: {stale}"
        );
    }
    assert!(shipped("skills/ivar-plan/references/task-template.md").contains("**Design ref:**"));
    assert!(shipped("skills/ivar-plan/SKILL.md").contains("On omp"));
    assert!(shipped("skills/ivar-deliver/SKILL.md").contains("## After delivery"));
}
