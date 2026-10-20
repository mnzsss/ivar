//! POST a finished run's summary to the agent's `notify` URL.

use std::time::Duration;

use crate::domain::agent::receipt::AgentRunReceipt;
use crate::domain::agent::spec::AgentSpec;
use crate::infra::{json, webhook};

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Records a failed delivery in `notify_error`; never changes the outcome.
pub fn deliver(spec: &AgentSpec, receipt: &mut AgentRunReceipt) {
    let Some(url) = spec.notify.as_deref() else {
        return;
    };
    let body = match json::to_canonical_string(&receipt.notify_summary()) {
        Ok(body) => body,
        Err(e) => {
            receipt.notify_error = Some(e.to_string());
            return;
        }
    };
    if let Err(e) = webhook::post_json(url, &body, NOTIFY_TIMEOUT) {
        receipt.notify_error = Some(e);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/notify.rs"]
mod tests;
