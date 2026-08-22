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

use crate::domain::name::FeatureName;

/// Build the session bootstrap block for `feature`, whose plan is reachable
/// from the View Dir at `plan_rel_path` (e.g. `plans/checkout/plan.md`).
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
2. Read the plan artifacts that exist under `plans/{feature}/` —
   `requirements.md`, `analysis.md`, `plan.md`.
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

The plan files are real: edits under `plans/{feature}/` land in the hall's
committed plan directory.
<!-- ivar:session:end -->"#
    )
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/session.rs"]
mod tests;
