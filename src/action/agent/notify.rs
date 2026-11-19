//! POST a finished run's summary to the agent's `notify` target.

use std::time::Duration;

use crate::action::read_manifest;
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, ProblemSummary};
use crate::domain::agent::spec::{AgentSpec, NotifyTarget};
use crate::domain::agent::telegram_card::COPY_PROMPT_LABEL;
use crate::infra::{json, telegram, webhook};
use crate::store::agent_secrets::AgentSecrets;
use crate::store::layout::Layout;

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);
pub const TELEGRAM_TOKEN_KEY: &str = "TELEGRAM_BOT_TOKEN";

/// Records a failed delivery in `notify_error`; never changes the outcome.
pub fn deliver(layout: &Layout, spec: &AgentSpec, receipt: &mut AgentRunReceipt) {
    let hall = read_manifest(layout).map_or_else(
        |_| layout.root().file_name().unwrap_or_default().to_owned(),
        |manifest| manifest.name().to_string(),
    );
    deliver_to(
        spec,
        receipt,
        &hall,
        || telegram_token(layout),
        telegram::API,
    );
}

pub(crate) fn deliver_to(
    spec: &AgentSpec,
    receipt: &mut AgentRunReceipt,
    hall: &str,
    token: impl FnOnce() -> Result<String, String>,
    telegram_api: &str,
) {
    let Some(target) = &spec.notify else { return };
    if let Err(e) = send(target, receipt, hall, token, telegram_api, false) {
        receipt.notify_error = Some(e);
    }
}

/// Tells the hall's `agents.problem_notify` target about a `blocked` run: a
/// failed provider preflight, or a run that needs a person. Skipped when
/// `ivar.json` sets none, when the run is anything else, or when it is the
/// target the agent's own `notify` already told. A failed delivery goes in
/// `notify_error`, after any the agent's own `notify` left; it never changes
/// the outcome.
pub fn problem(layout: &Layout, spec: &AgentSpec, receipt: &mut AgentRunReceipt) {
    if receipt.outcome != AgentRunOutcome::Blocked {
        return;
    }
    let Ok(manifest) = read_manifest(layout) else {
        return;
    };
    let hall = manifest.name().to_string();
    problem_to(
        manifest.agent_problem_notify().as_ref(),
        spec,
        receipt,
        &hall,
        || telegram_token(layout),
        telegram::API,
    );
}

pub(crate) fn problem_to(
    target: Option<&NotifyTarget>,
    spec: &AgentSpec,
    receipt: &mut AgentRunReceipt,
    hall: &str,
    token: impl FnOnce() -> Result<String, String>,
    telegram_api: &str,
) {
    let Some(target) = target else { return };
    if receipt.outcome != AgentRunOutcome::Blocked || spec.notify.as_ref() == Some(target) {
        return;
    }
    if let Err(e) = send(target, receipt, hall, token, telegram_api, true) {
        let e = format!("problem notifier: {e}");
        receipt.notify_error = Some(match receipt.notify_error.take() {
            Some(earlier) => format!("{earlier}; {e}"),
            None => e,
        });
    }
}

fn send(
    target: &NotifyTarget,
    receipt: &AgentRunReceipt,
    hall: &str,
    token: impl FnOnce() -> Result<String, String>,
    telegram_api: &str,
    problem: bool,
) -> Result<(), String> {
    match target {
        NotifyTarget::Webhook(url) => post_summary(url, receipt, hall, problem),
        NotifyTarget::Telegram { chat_id } => token().and_then(|token| {
            let summary = receipt.notify_summary(hall);
            telegram::send_card(
                telegram_api,
                &token,
                chat_id,
                &summary.telegram_html(),
                &summary.telegram_text(),
                (COPY_PROMPT_LABEL, &summary.investigation_prompt()),
            )
        }),
    }
}

fn post_summary(
    url: &str,
    receipt: &AgentRunReceipt,
    hall: &str,
    problem: bool,
) -> Result<(), String> {
    let summary = receipt.notify_summary(hall);
    let body = if problem {
        json::to_canonical_string(&ProblemSummary::new(summary))
    } else {
        json::to_canonical_string(&summary)
    }
    .map_err(|e| e.to_string())?;
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
