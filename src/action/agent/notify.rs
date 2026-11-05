//! POST a finished run's summary to the agent's `notify` target.

use std::time::Duration;

use crate::domain::agent::receipt::AgentRunReceipt;
use crate::domain::agent::spec::{AgentSpec, NotifyTarget};
use crate::infra::{json, telegram, webhook};
use crate::store::agent_secrets::AgentSecrets;
use crate::store::layout::Layout;

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);
pub const TELEGRAM_TOKEN_KEY: &str = "TELEGRAM_BOT_TOKEN";

/// Records a failed delivery in `notify_error`; never changes the outcome.
pub fn deliver(layout: &Layout, spec: &AgentSpec, receipt: &mut AgentRunReceipt) {
    deliver_to(spec, receipt, || telegram_token(layout), telegram::API);
}

pub(crate) fn deliver_to(
    spec: &AgentSpec,
    receipt: &mut AgentRunReceipt,
    token: impl FnOnce() -> Result<String, String>,
    telegram_api: &str,
) {
    let result = match &spec.notify {
        None => return,
        Some(NotifyTarget::Webhook(url)) => post_summary(url, receipt),
        Some(NotifyTarget::Telegram { chat_id }) => token().and_then(|token| {
            let text = receipt.notify_summary().telegram_text();
            telegram::send_message(telegram_api, &token, chat_id, &text)
        }),
    };
    if let Err(e) = result {
        receipt.notify_error = Some(e);
    }
}

fn post_summary(url: &str, receipt: &AgentRunReceipt) -> Result<(), String> {
    let body = json::to_canonical_string(&receipt.notify_summary()).map_err(|e| e.to_string())?;
    webhook::post_json(url, &body, NOTIFY_TIMEOUT)
}

fn telegram_token(layout: &Layout) -> Result<String, String> {
    AgentSecrets::read(layout)
        .map_err(|failure| failure.what)?
        .get(TELEGRAM_TOKEN_KEY)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("secret `{TELEGRAM_TOKEN_KEY}` is not set in .ivar/secrets/agents.env")
        })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/notify.rs"]
mod tests;
