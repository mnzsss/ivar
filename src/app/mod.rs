//! The binary's composition root — dispatch and rendering — the only layer
//! allowed to reach every other one.

mod agent_dispatch;
mod graph_dispatch;
mod notice;
mod respond;
pub mod run;
