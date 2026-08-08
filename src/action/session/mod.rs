//! `ivar session` — open a session: a view dir over a feature's promoted
//! repos, an agent running in it, and the TUI that ties them together.
//!
//! This slice's additions: `connect` (re-bind to a live session),
//! `conversion` (bind a discovery session to a feature, one-way), and the
//! shared `lookup` (finding sessions on disk by id-prefix and/or feature).

pub mod connect;
pub mod conversion;
pub(crate) mod lookup;
pub mod start;
