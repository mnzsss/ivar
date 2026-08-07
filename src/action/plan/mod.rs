//! `ivar plan` — SPDD planning artifacts for a feature.
//!
//! The SPDD process produces three committed Markdown files per feature,
//! under `<hall>/plans/<feature>/`: `requirements.md`, `analysis.md`, and
//! `plan.md`. This module manages those files on disk — create, show, and
//! list — and the four approval gates around them (`approve` /
//! `invalidate`): the gates are crossed by explicit commands, recorded per
//! feature at `features/<feature>/planning/approvals.json`, and invalidated
//! by a change to an upstream artifact.
//!
//! The files are committed (they are the team's shared record of *why* a
//! feature exists), which is why the layout puts them at the hall root under
//! `plans/`, not under `.ivar/`.

pub mod approve;
pub mod create;
pub mod list;
pub mod show;
