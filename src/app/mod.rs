//! The binary's composition root — dispatch and rendering — the only layer
//! allowed to reach every other one.

mod agent_dispatch;
mod feature_dispatch;
mod graph_dispatch;
mod notice;
mod planning_dispatch;
mod respond;
pub mod run;
mod session_dispatch;
mod skill_dispatch;
