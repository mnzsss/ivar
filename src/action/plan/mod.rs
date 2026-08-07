//! `ivar plan` — SPDD planning artifacts for a feature.
//!
//! The SPDD process produces three committed Markdown files per feature,
//! under `<hall>/plans/<feature>/`: `requirements.md`, `analysis.md`, and
//! `plan.md`. This module manages those files on disk — create, show, and
//! list. It deliberately does **not** implement approval gates or generate
//! plan content: the artifacts are the *input* to a planning conversation
//! with an agent, and the gates around them are a later slice's concern.
//!
//! The files are committed (they are the team's shared record of *why* a
//! feature exists), which is why the layout puts them at the hall root under
//! `plans/`, not under `.ivar/`.

pub mod create;
pub mod list;
pub mod show;
