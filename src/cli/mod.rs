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
//! Those conversions live here and nowhere else — `bin/ivar.rs` is pure
//! dispatch — and each one destructures its args struct exhaustively, so a
//! declared flag that nothing forwards is a compile error rather than help
//! text for a no-op. See ARCHITECTURE.md, seam 8.
//!
//! `cli` may import `action` and `error` only.

pub mod root;
