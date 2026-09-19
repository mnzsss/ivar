//! clap derive types ONLY — structs, enums, doc comments.
//!
//! Parsing raw `argv` into these types is clap's job. Converting a parsed
//! args struct into an `action::*Input` is the one bit of code allowed here,
//! and it is a straight shape conversion — no validation, no I/O. Validating
//! a value (an unknown `--provider`, a malformed `--name`) needs `domain`,
//! which this module may not import (see the layering table in
//! ARCHITECTURE.md and `tests/architecture.rs`, which enforces it), so that
//! work belongs to the `action` function the converted `Input` is handed to.
//!
//! Those conversions live here and nowhere else — `run` is pure dispatch —
//! and each one destructures its args struct exhaustively, so a declared
//! flag that nothing forwards is a compile error rather than help text for
//! a no-op. See ARCHITECTURE.md, seam 8.
//!
//! `root` and `graph` (the clap derive types) may import `action` only.
//! `run` and `graph_dispatch` (dispatch, moved here from `bin/ivar.rs`) may
//! additionally import `domain`, `infra`, and `git` — the same access the
//! binary crate had, since they still only render an action's `Outcome` and
//! never validate a value themselves. See `tests/architecture.rs`, which
//! enforces both halves of this rule.

pub mod graph;
mod graph_dispatch;
mod respond;
pub mod root;
pub mod run;

pub use graph::{GraphArgs, GraphCommand};
