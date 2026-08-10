//! Unit tests for `crate::action::execute::plan_ops`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::indexing_slicing)]

use super::*;

const REVISED_PLAN: &str = "# Plan\n\
    \n\
    ## Operations\n\
    \n\
    ### ws-gates\n\
    - add-gate-types\n\
    - wire-approve\n\
    write_contract:\n\
    - src/domain/feature.rs\n\
    \n\
    ### ws-board\n\
    - add-board-types\n\
    - store-board\n\
    - tick-board\n\
    write_contract:\n\
    - src/action/execute\n";

#[test]
fn operations_from_plan_parses_ids_operations_and_write_contracts() {
    let parsed = operations_from_plan(REVISED_PLAN);

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
    assert!(operations_from_plan("# Plan\n\nprose only\n").is_empty());
}
