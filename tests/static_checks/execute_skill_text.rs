//! Text checks over the shipped `ivar-execute` skill.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::shipped::shipped;

/// Shipped skill `ivar-execute` aligns with the execution run receipt lifecycle
/// and completion branching: starting the run in preparation, finishing before
/// completion, and branching to integrate for subfeatures or deliver for root features.
#[test]
fn ivar_execute_skill_contains_execute_start_and_finish_commands() {
    let skill_path = format!(
        "{}/src/harness/skills/ivar-execute/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(skill_path).unwrap();

    assert!(
        content.contains("ivar feature execute start"),
        "skill must document starting the run receipt during preparation"
    );
    assert!(
        content.contains("ivar feature execute finish"),
        "skill must document finishing the run receipt before integration/delivery"
    );
    assert!(
        content.contains("ivar feature integrate"),
        "skill must document integrating subfeatures upon completion"
    );
    assert!(
        content.contains("ivar feature deliver"),
        "skill must document delivering root features upon completion"
    );
    assert!(
        content.contains("is_subfeature"),
        "skill must document branching on subfeature status"
    );
}

#[test]
fn ivar_execute_skill_finishes_the_run_with_a_report_and_outcome() {
    let skill_path = format!(
        "{}/src/harness/skills/ivar-execute/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(skill_path).unwrap();

    assert!(
        content.contains("ivar feature execute finish <feature> --report-json <path> --outcome"),
        "skill must show the flags finish requires"
    );
    assert!(
        content.contains("ivar feature execute finish --print-schema"),
        "skill must point at the report schema"
    );
}

#[test]
fn ivar_execute_skill_delivers_root_features_through_the_deliver_skill() {
    let skill_path = format!(
        "{}/src/harness/skills/ivar-execute/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(skill_path).unwrap();

    assert!(
        content.contains("Load the `ivar-deliver` skill"),
        "root feature delivery must hand off to the ivar-deliver skill"
    );
}

/// Wave progress goes into the run receipt; editing `plan.md` mid-run moves
/// its fingerprint and diverges the run.
#[test]
fn ivar_execute_skill_records_progress_through_execute_checkpoint_only() {
    let base = env!("CARGO_MANIFEST_DIR");
    let execute =
        std::fs::read_to_string(format!("{base}/src/harness/skills/ivar-execute/SKILL.md"))
            .unwrap();
    let plan =
        std::fs::read_to_string(format!("{base}/src/harness/skills/ivar-plan/SKILL.md")).unwrap();

    assert!(execute.contains("ivar feature execute checkpoint"));
    assert!(execute.contains("Never edit `plan.md` during a run"));
    for stale in [
        "records progress and deferred validation failures in `plan.md`",
        "record in plan.md",
        "record completed wave in plan.md",
        "for this wave in `plan.md`",
        "update `plan.md`",
    ] {
        assert!(!execute.contains(stale), "ivar-execute still says: {stale}");
    }
    assert!(!plan.contains("marks each wave complete in"));
}

#[test]
fn execute_and_deliver_skills_forbid_ai_attribution() {
    for skill in ["ivar-execute", "ivar-deliver"] {
        let source = format!(
            "{}/src/harness/skills/{skill}/SKILL.md",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(source).unwrap();
        assert!(
            body.contains("Never add AI attribution"),
            "{skill} should forbid AI attribution"
        );
    }
    let deliver = std::fs::read_to_string(format!(
        "{}/src/harness/skills/ivar-deliver/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(
        deliver.contains("deliver.ai_attribution"),
        "deliver should name the refusal code"
    );
}

#[test]
fn every_review_loop_in_execute_has_a_round_cap() {
    let execute = shipped("skills/ivar-execute/SKILL.md");
    assert!(
        !execute.contains("until 0 findings"),
        "goal mode must not demand zero smells"
    );
    assert!(
        execute.matches("at most").count() >= 2,
        "default and goal review loops both need a cap"
    );
    assert!(!execute.contains('┌'), "the diagram duplicates Phases 1-4");
    assert!(
        execute.contains("## Recovery"),
        "execute must document recovery"
    );
    for case in ["usage limit", "subagent", "CI fails after deliver"] {
        assert!(execute.contains(case), "recovery must cover: {case}");
    }
    assert!(shipped("skills/ivar-execute/references/subagent.md").contains("Design ref"));
}

#[test]
fn execute_recovery_covers_two_live_sessions() {
    let execute = shipped("skills/ivar-execute/SKILL.md");
    assert!(execute.contains("**Two live sessions on the feature:**"));
    assert!(execute.contains("`$IVAR_SESSION_ID`"));
}

#[test]
fn ivar_execute_skill_supports_explicit_feature_and_orchestrator_handoff() {
    let execute = shipped("skills/ivar-execute/SKILL.md");

    // Frontmatter argument-hint must take optional feature
    assert!(
        execute.contains("argument-hint: [feature] [plan-path] [--mode goal|default]"),
        "ivar-execute must declare [feature] in argument-hint"
    );

    // Phase 1.1 feature resolution & plan default
    assert!(
        execute.contains("Resolve the target feature from `$ARGUMENTS`"),
        "Phase 1.1 must resolve target feature from arguments with fallback"
    );
    assert!(
        execute.contains("`.ivar/features/<feature>/plan.md`"),
        "Phase 1.1 default plan must be relative to hall root"
    );

    // Phase 4.2 report path
    assert!(
        execute.contains(".tmp/<feature>-run-report.json"),
        "Phase 4.2 report filename must include feature name"
    );

    // Phase 4.4 hand-off branching for child-session vs parent orchestrator
    assert!(
        execute.contains("If this session belongs to the parent"),
        "Phase 4.4 must handle orchestrator returning control to ivar-subfeatures"
    );
    assert!(
        execute.contains("Stop this session, then integrate from the parent session"),
        "Phase 4.4 must retain stop instruction when session belongs to child"
    );

    // Recovery detached session startup
    assert!(
        execute.contains("Root feature driven by `ivar-subfeatures`"),
        "Phase 4.4 must return a parent Wave 0 run to the orchestrator"
    );
    assert!(
        execute.contains("ivar session start <feature> --detached"),
        "Recovery must instruct orchestrator to start detached child session"
    );
}

#[test]
fn execute_delegates_only_to_the_claude_provider_and_never_isolates_in_a_t3_worktree() {
    let execute = shipped("skills/ivar-execute/SKILL.md");
    assert!(execute.contains("`delegate_task`"));
    assert!(execute.contains("`claudeAgent`"));
    assert!(execute.contains("never set `isolation`"));
}
