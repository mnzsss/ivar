//! `ivar session` — open a session: a view dir over a feature's promoted
//! repos, an agent running in it, and the TUI that ties them together.
//!
//! This slice's additions: `connect` (re-bind to a live session),
//! `conversion` (bind a discovery session to a feature, one-way), the shared
//! `lookup` (finding sessions on disk by id-prefix and/or feature), the
//! lifecycle pair `stop` (end a detached session) and `prune` (remove dead
//! sessions), and the `relay` verb (a thin alias over `start --relay`).

pub mod connect;
pub mod conversion;
pub(crate) mod hook;
pub(crate) mod lookup;
pub mod prune;
pub mod relay;
pub mod start;
pub mod stop;
pub(crate) mod view;
