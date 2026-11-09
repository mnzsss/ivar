//! Text checks over the shipped `ivar-subfeatures` skill.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::shipped::shipped;

#[test]
fn subfeatures_orchestrates_children_with_subagents_from_the_parent_session() {
    let skill = shipped("skills/ivar-subfeatures/SKILL.md");
    // The manual one-terminal-per-child flow is gone.
    assert!(!skill.contains("ivar session sandbox --session"));
    assert!(!skill.contains("never launch a provider yourself"));
    // Portable subagent contract: one level, return-only.
    for rule in [
        "One level of subagents",
        "never message a running subagent",
        "Absolute paths only",
        "needs_input",
        "Wave 0",
        "references/child-planner.md",
        "references/state.md",
        "ivar-execute/references/subagent.md",
    ] {
        assert!(skill.contains(rule), "subfeatures must state: {rule}");
    }
    // Every provider names its dispatch mechanism.
    for provider in ["Claude Code", "OpenCode", "omp"] {
        assert!(skill.contains(provider), "dispatch for {provider}");
    }
    // The child lifecycle, in order.
    let order = [
        "ivar feature create <child> --parent <parent>",
        "ivar plan approve <child> plan",
        "ivar session start <child> --detached --json",
        "ivar feature execute start <child> --mode goal",
        "ivar feature execute finish <child>",
        "ivar session stop <session-id>",
        "ivar feature integrate <child>",
        "ivar feature deliver <parent> --preview",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|step| {
            skill
                .find(step)
                .unwrap_or_else(|| panic!("missing: {step}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "lifecycle out of order: {positions:?}"
    );
    // It stops before applying delivery.
    assert!(!skill.contains("ivar feature deliver <parent> --fingerprint"));

    let planner = shipped("skills/ivar-subfeatures/references/child-planner.md");
    for field in [
        "status: done",
        "status: needs_input",
        "questions:",
        "### Decisions",
        "absolute",
    ] {
        assert!(
            planner.contains(field),
            "child-planner must define: {field}"
        );
    }
    let state = shipped("skills/ivar-subfeatures/references/state.md");
    for column in [
        "plan_gate",
        "run",
        "sessions",
        "last_wave",
        "needs_revision",
        "diverged",
    ] {
        assert!(state.contains(column), "state table must read: {column}");
    }
}

#[test]
fn subfeatures_dispatches_a_t3_session_like_claude_code() {
    let subfeatures = shipped("skills/ivar-subfeatures/SKILL.md");
    assert!(subfeatures.contains(
        "| t3 (Claude Code in T3 Code) | the Task tool with a general-purpose subagent |"
    ));
}
