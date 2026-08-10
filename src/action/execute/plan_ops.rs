//! Parse a plan.md's `## Operations` section into per-workstream operation
//! lists and write contracts.
//!
//! # Why this is its own module
//!
//! `operations_from_plan` was originally private to
//! [`super::replan`], the only verb that needed it. The executor prompt
//! renderer ([`super::prompt`]) needs the same parse — which workstream owns
//! which operations, per the plan's own Operations section — to know what to
//! put in an agent's hands. Copy-pasting the parser into a second module
//! would let the two forks drift on what counts as a heading, a bullet, or
//! the write-contract marker, and the format has a sharp edge (the section
//! never ends once entered — see `replan`'s module doc) that is exactly the
//! kind of behaviour a fork silently loses. One parser, two callers.
//!
//! This module is a pure relocation: the parsing logic below is unchanged
//! from `replan.rs`, only made visible to the rest of the crate.

/// One workstream's Operations as authored in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanWorkstream {
    /// The workstream's id — the subheading text under `Operations`.
    pub(crate) id: String,
    /// The operations, in order.
    pub(crate) operations: Vec<String>,
    /// The paths the workstream may touch.
    pub(crate) write_contract: Vec<String>,
}

/// Parse `text`'s Operations section. See [`super::replan`]'s module doc
/// comment for the exact format; a plan without an Operations section yields
/// an empty list, which makes every board workstream affected — the
/// conservative answer when the new plan carries no operations at all.
pub(crate) fn operations_from_plan(text: &str) -> Vec<PlanWorkstream> {
    let mut workstreams = Vec::new();
    let mut in_operations = false;
    let mut collecting_write_contract = false;
    let mut current: Option<PlanWorkstream> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(heading) = trimmed.strip_prefix('#') {
            let title = heading.trim_start_matches('#').trim();
            if title.eq_ignore_ascii_case("operations") {
                // The section (re)starts; whatever workstream was open ends.
                if let Some(workstream) = current.take() {
                    workstreams.push(workstream);
                }
                in_operations = true;
                collecting_write_contract = false;
                continue;
            }
            if !in_operations {
                continue;
            }
            // Any other heading inside the section starts a new workstream,
            // named by the heading text.
            if let Some(workstream) = current.take() {
                workstreams.push(workstream);
            }
            current = Some(PlanWorkstream {
                id: title.to_owned(),
                operations: Vec::new(),
                write_contract: Vec::new(),
            });
            collecting_write_contract = false;
            continue;
        }

        if !in_operations {
            continue;
        }
        let Some(workstream) = current.as_mut() else {
            continue;
        };
        if trimmed == "write_contract:" {
            collecting_write_contract = true;
            continue;
        }
        if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = bullet.trim().to_owned();
            if collecting_write_contract {
                workstream.write_contract.push(item);
            } else {
                workstream.operations.push(item);
            }
        }
    }
    if let Some(workstream) = current {
        workstreams.push(workstream);
    }

    workstreams
}

#[cfg(test)]
mod tests {
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
}
