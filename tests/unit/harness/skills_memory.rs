const EXPLORE: &str = include_str!("../../../src/harness/skills/ivar-explore/SKILL.md");
const PLAN: &str = include_str!("../../../src/harness/skills/ivar-plan/SKILL.md");
const TASK_TEMPLATE: &str =
    include_str!("../../../src/harness/skills/ivar-plan/references/task-template.md");
const EXECUTE: &str = include_str!("../../../src/harness/skills/ivar-execute/SKILL.md");
const REVIEW: &str = include_str!("../../../src/harness/commands/review.md");
const SUBAGENT: &str =
    include_str!("../../../src/harness/skills/ivar-execute/references/subagent.md");

#[test]
fn explore_queries_memory_once_and_records_prior_knowledge_in_the_discovery() {
    assert!(EXPLORE.contains("memory_query"));
    assert!(EXPLORE.contains("ivar memory query"));
    assert!(EXPLORE.contains("Conhecimento prévio"));
    assert!(EXPLORE.contains("ivar discovery amend"));
    assert!(EXPLORE.contains("memória indisponível"));
    assert!(
        EXPLORE.contains("one question per turn") || EXPLORE.contains("one question at a time")
    );
}

#[test]
fn plan_writes_one_decision_per_trade_off_when_the_analysis_is_approved() {
    assert!(PLAN.contains("memory_write_page"));
    assert!(PLAN.contains("one `decision` per trade-off"));
    assert!(PLAN.contains("docs/memory/decision/"));
}

#[test]
fn task_packets_carry_memory_refs_beside_the_design_ref() {
    assert!(TASK_TEMPLATE.contains("**Memory refs:**"));
    assert!(SUBAGENT.contains("Memory refs"));
    assert!(SUBAGENT.contains("Learnings"));
    assert!(SUBAGENT.contains("never call `memory_write_page`"));
}

#[test]
fn execute_forwards_memory_refs_to_reviewers_and_keeps_writes_with_the_coordinator() {
    assert!(EXECUTE.contains("**Memory refs:**"));
    assert!(EXECUTE.contains("both reviewers"));
    assert!(EXECUTE.contains("Standards"));
    assert!(EXECUTE.contains("only the coordinator writes"));
    assert!(EXECUTE.contains("`gotcha`"));
}

#[test]
fn the_edited_skills_fit_the_commit_hook_limit() {
    for text in [EXPLORE, PLAN, EXECUTE, SUBAGENT, TASK_TEMPLATE] {
        assert!(text.len() < 20_000);
    }
}

#[test]
fn reviews_always_ask_whether_follow_ups_land_in_the_same_feature_and_record_the_answer() {
    for (name, text) in [("ivar-execute", EXECUTE), ("ivar-review", REVIEW)] {
        assert!(text.contains("land in this same feature"), "{name}");
        assert!(
            text.contains("tags: [follow-ups]") || text.contains("tag `follow-ups`"),
            "{name}"
        );
        assert!(text.contains("Follow-up preference"), "{name}");
    }
    assert!(EXECUTE.contains("Goal mode applies the `Follow-up preference` procedure"));
}
