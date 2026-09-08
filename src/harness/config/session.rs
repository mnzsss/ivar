//! The session bootstrap block: what an agent must do at the start of a
//! session to re-derive where the feature is in the SPDD cycle.
//!
//! A session's View Dir carries a provider-native instruction file
//! (`CLAUDE.md` / `AGENTS.md`) built by `action::session::view` as this block
//! followed by the hall's standing instructions. The block is the session's
//! continuation contract — it is what lets a relay (or a fresh conversation on
//! an existing session) pick the feature's work back up.
//!
//! # Derived, never stored
//!
//! The block tells the agent to compute the planning and Run Receipt state with
//! `ivar plan status` and `ivar feature execute status`. It does not record
//! either state: those durable records remain the source of truth, and this
//! file is a pure builder — no I/O, no clock — so identical inputs produce
//! identical bytes.

use crate::domain::memory::context::{MEMORY_MANAGED_END, MEMORY_MANAGED_START};
use crate::domain::name::FeatureName;

/// Build the session bootstrap block for `feature`, whose plan is reachable
/// from the View Dir at `plan_rel_path` (e.g. `../../plan.md`).
#[must_use]
pub(crate) fn build_session_block(feature: &FeatureName, plan_rel_path: &str) -> String {
    format!(
        r#"<!-- ivar:session:start -->
# ivar session — feature `{feature}`

This View Dir is a session on feature `{feature}`. The work lives on disk —
the plan, the branches, the promotion records; the conversation that started
it is gone. A relay preserves the work, never the thread.

Before proposing or editing anything, re-derive planning state:

1. Run `ivar plan status {plan_rel_path}`.
2. Read the working documents in the feature directory (two levels up) —
   `../../requirements.md`, `../../analysis.md`, `{plan_rel_path}`.
3. Continue from the first approval gate that is `pending` or
   `needs-revision`. A `needs-revision` gate means its artifact changed since
   it was approved: revise the artifact, then re-approve the gate with
   `ivar plan approve {feature} <gate>`.
4. New human approval gates can appear at any time — pause and wait for them.

When the Plan gate is approved, inspect the current Run Receipt before acting:

```sh
ivar feature execute status {feature}
```

- No receipt or a terminal receipt: begin execution with
  `ivar feature execute start {feature} --plan {plan_rel_path}`.
- `active` or `blocked`: continue the logical run with
  `ivar feature execute start {feature} --plan {plan_rel_path} --resume`.
- `diverged`: inspect the approved revision; use
  `ivar feature execute accept-revision {feature} --plan {plan_rel_path}`
  before resuming, or use `--restart` when a fresh run is appropriate.
- To abandon any non-terminal run and begin again, use
  `ivar feature execute start {feature} --plan {plan_rel_path} --restart`.

The working documents are real: edits in the feature directory land in
the hall's feature working directory (`.ivar/features/{feature}/`).
<!-- ivar:session:end -->"#
    )
}

/// Compose instructions with a rendered memory block.
///
/// If `instructions` contains `<!-- ivar:memory:start -->` and
/// `<!-- ivar:memory:end -->`, the region between (and including) those markers
/// is replaced with `memory_block`, preserving all content outside byte-exact.
///
/// If `instructions` does not contain the markers and `memory_block` is non-empty,
/// `memory_block` is appended to `instructions`.
#[must_use]
pub(crate) fn compose_instructions_with_memory(instructions: &str, memory_block: &str) -> String {
    if let Some(start_idx) = instructions.find(MEMORY_MANAGED_START) {
        if let Some(end_rel) = instructions[start_idx..].find(MEMORY_MANAGED_END) {
            let end_idx = start_idx + end_rel + MEMORY_MANAGED_END.len();
            let before = &instructions[..start_idx];
            let after = &instructions[end_idx..];
            return if memory_block.is_empty() {
                format!("{before}{after}")
            } else {
                format!("{before}{memory_block}{after}")
            };
        }
    }

    if memory_block.is_empty() {
        return instructions.to_string();
    }

    if instructions.is_empty() {
        memory_block.to_string()
    } else {
        format!("{instructions}\n\n{memory_block}")
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/session.rs"]
mod tests;
