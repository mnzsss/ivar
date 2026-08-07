//! `ivar feature execute` — the feature execution board.
//!
//! v1 implements exactly one verb: `prepare`, which turns a feature's plan
//! and execution graph into an [`ExecutionBoard`] on disk. Everything that
//! advances a board — tick, reply, workstream status — is v2; v1 only
//! creates the board.

pub mod prepare;
