//! The record of an agent run that is going on right now.

use serde::{Deserialize, Serialize};

use super::receipt::Trigger;

/// Written when a run takes its agent's lock and removed when it lets go.
///
/// `pid` is the process holding the lock. `pid_start` is that process's start
/// time where the platform reports one, so a record left by a run that died
/// is not mistaken for a live run when the kernel hands its pid to someone else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentRun {
    pub run_id: String,
    pub trigger: Trigger,
    pub started_at: String,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_start: Option<String>,
}
