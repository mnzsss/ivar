//! Unit tests for `crate::action::execute::plan_ops`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

const REVISED_PLAN: &str = "# Plan\n\
    \n\
    ## Operations\n\
    \n\
    ### ws-gates\n\
    provider: opencode\n\
    model: deepseek-chat\n\
    agent: implementer-deepseek\n\
    - add-gate-types\n\
    - wire-approve\n\
    write_contract:\n\
    - src/domain/feature.rs\n\
    \n\
    ### ws-board\n\
    provider: claude-code\n\
    - add-board-types\n\
    - store-board\n\
    - tick-board\n\
    write_contract:\n\
    - src/action/execute\n";

#[test]
fn operations_from_plan_parses_ids_operations_and_write_contracts() {
    let parsed = operations_from_plan(REVISED_PLAN).unwrap();

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].id, "ws-gates");
    assert_eq!(
        parsed[0].operations,
        vec!["add-gate-types".to_owned(), "wire-approve".to_owned()]
    );
    assert_eq!(
        parsed[0].write_contract,
        vec!["src/domain/feature.rs".to_owned()]
    );
    assert_eq!(parsed[1].id, "ws-board");
    assert_eq!(parsed[1].operations.len(), 3);

    // A plan with no Operations section parses to nothing — and every
    // board workstream therefore counts as affected.
    assert!(
        operations_from_plan("# Plan\n\nprose only\n")
            .unwrap()
            .is_empty()
    );
}

/// Scalar targeting lines (`provider:`, `model:`, `agent:`) parse alongside
/// the operation bullets, in the same block, before the write contract.
#[test]
fn operations_from_plan_parses_scalar_targeting_lines() {
    let parsed = operations_from_plan(REVISED_PLAN).unwrap();

    assert_eq!(parsed[0].provider, Some(Provider::OpenCode));
    assert_eq!(parsed[0].model.as_deref(), Some("deepseek-chat"));
    assert_eq!(parsed[0].agent.as_deref(), Some("implementer-deepseek"));
    assert_eq!(parsed[1].provider, Some(Provider::ClaudeCode));
    assert_eq!(parsed[1].model, None);
    assert_eq!(parsed[1].agent, None);
}

/// An unknown provider id is refused here, naming the bad id and the valid
/// options — the same treatment `--provider` and the graph JSON give it.
#[test]
fn operations_from_plan_refuses_an_unknown_provider() {
    let plan = "# Plan\n\n## Operations\n\n### ws-a\nprovider: deepseek\n- op-a\n";
    let failure = operations_from_plan(plan).unwrap_err();
    assert_eq!(failure.code, "execute.plan_provider_invalid");
    assert!(failure.what.contains("deepseek"));
    assert!(failure.what.contains("claude-code"));
    assert!(failure.what.contains("opencode"));
}

/// An empty `model:` or `agent:` value is a refused authoring mistake, not a
/// silently dropped selector.
#[test]
fn operations_from_plan_refuses_an_empty_targeting_value() {
    let plan = "# Plan\n\n## Operations\n\n### ws-a\nmodel:\n- op-a\n";
    let failure = operations_from_plan(plan).unwrap_err();
    assert_eq!(failure.code, "execute.plan_target_empty");
    assert!(failure.what.contains("ws-a"));
    assert!(failure.what.contains("model"));
}

/// Rewriting targeting into a plan keeps every operation and the write
/// contract byte for byte, and leaves non-targeted workstreams untouched.
#[test]
fn write_targets_persists_resolved_targeting_without_changing_operations() {
    let targets = vec![ResolvedTarget {
        id: "ws-gates".to_owned(),
        provider: Provider::OpenCode,
        model: Some("deepseek-chat".to_owned()),
        agent: Some("implementer-deepseek".to_owned()),
    }];

    let rewritten = write_targets(REVISED_PLAN, &targets).unwrap();
    let parsed = operations_from_plan(&rewritten).unwrap();

    assert_eq!(parsed[0].provider, Some(Provider::OpenCode));
    assert_eq!(parsed[0].operations, vec!["add-gate-types", "wire-approve"]);
    assert!(rewritten.contains(
        "### ws-gates\nprovider: opencode\nmodel: deepseek-chat\nagent: implementer-deepseek"
    ));
    // The sibling workstream's own targeting and operations are untouched.
    assert!(rewritten.contains("### ws-board\nprovider: claude-code"));
    assert_eq!(
        parsed[1].operations,
        vec!["add-board-types", "store-board", "tick-board"]
    );
}

/// A stale `provider:` line inside a targeted block is replaced, not
/// duplicated — the resolved value is the only one that survives.
#[test]
fn write_targets_replaces_existing_targeting_lines_in_the_block() {
    let plan =
        "# Plan\n\n## Operations\n\n### ws-a\nprovider: claude-code\nmodel: old-model\n- op-a\n";
    let targets = vec![ResolvedTarget {
        id: "ws-a".to_owned(),
        provider: Provider::OpenCode,
        model: None,
        agent: None,
    }];

    let rewritten = write_targets(plan, &targets).unwrap();

    assert!(rewritten.contains("### ws-a\nprovider: opencode\n- op-a"));
    assert!(!rewritten.contains("claude-code"));
    assert!(!rewritten.contains("old-model"));
}

/// A target with no matching `###` heading in the Operations section is a
/// refusal, not a silent drop — the plan and the graph must name the same
/// workstreams.
#[test]
fn write_targets_refuses_a_target_with_no_matching_heading() {
    let plan = "# Plan\n\n## Operations\n\n### ws-a\n- op-a\n";
    let targets = vec![ResolvedTarget {
        id: "ws-ghost".to_owned(),
        provider: Provider::OpenCode,
        model: None,
        agent: None,
    }];

    let failure = write_targets(plan, &targets).unwrap_err();

    assert_eq!(failure.code, "execute.plan_workstream_missing");
    assert!(failure.what.contains("ws-ghost"));
}
