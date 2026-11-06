//! What one scheduled-agent run records, and the rules that classify it.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::domain::session::epoch_secs_from_rfc3339;

pub const RECEIPT_VERSION: u32 = 1;

const NOTIFY_TAIL_CHARS: usize = 2000;
/// Telegram counts a message's length in UTF-16 code units.
pub(super) const TELEGRAM_TEXT_LIMIT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunOutcome {
    Succeeded,
    Failed,
    Blocked,
    Skipped,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    Schedule,
    Manual,
}

impl AgentRunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
            Self::TimedOut => "timed_out",
        }
    }
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Manual => "manual",
        }
    }
}

impl FromStr for Trigger {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "schedule" => Ok(Self::Schedule),
            "manual" => Ok(Self::Manual),
            other => Err(format!(
                "unknown trigger `{other}`; expected schedule or manual"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    Up,
    Down,
}

impl FromStr for Rating {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            other => Err(format!("unknown rating `{other}`; expected up or down")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunFeedback {
    pub rating: Rating,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRunReceipt {
    pub version: u32,
    pub run_id: String,
    pub agent: String,
    pub trigger: Trigger,
    pub started_at: String,
    pub ended_at: String,
    pub outcome: AgentRunOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<RunFeedback>,
}

#[derive(Debug, Serialize)]
pub struct NotifySummary<'a> {
    pub hall: &'a str,
    pub run_id: &'a str,
    pub agent: &'a str,
    pub trigger: Trigger,
    pub outcome: AgentRunOutcome,
    pub exit_code: Option<i32>,
    pub started_at: &'a str,
    pub ended_at: &'a str,
    pub tail: &'a str,
    #[serde(skip)]
    pub tail_is_cut: bool,
}

impl NotifySummary<'_> {
    /// Duration and a non-zero exit code; exit 0 repeats what `succeeded` says.
    pub(super) fn run_details(&self) -> String {
        let exit = self
            .exit_code
            .filter(|code| *code != 0)
            .map(|code| format!("exit {code}"));
        [self.duration(), exit]
            .into_iter()
            .flatten()
            .map(|part| format!(" · {part}"))
            .collect()
    }

    fn duration(&self) -> Option<String> {
        let started = epoch_secs_from_rfc3339(self.started_at)?;
        let seconds = epoch_secs_from_rfc3339(self.ended_at)? - started;
        match seconds {
            ..=0 => None,
            1..=59 => Some(format!("{seconds}s")),
            60..=3599 => Some(format!("{}m{:02}s", seconds / 60, seconds % 60)),
            _ => Some(format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60)),
        }
    }

    pub fn telegram_text(&self) -> String {
        let details = self.run_details();
        let mut text = format!(
            "ivar agent {}: {}\n{} · {} · {}{details}",
            self.agent,
            self.outcome.as_str(),
            self.hall,
            self.trigger.as_str(),
            self.run_id
        );
        if !self.tail.is_empty() {
            text.push_str("\n\n");
            let room = TELEGRAM_TEXT_LIMIT.saturating_sub(text.encode_utf16().count());
            text.push_str(utf16_suffix(self.tail, room));
        }
        text
    }
}

pub(super) fn utf16_suffix(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (start, ch) in text.char_indices().rev() {
        units += ch.len_utf16();
        if units > max_units {
            return &text[start + ch.len_utf8()..];
        }
    }
    text
}

impl AgentRunReceipt {
    fn new(
        run_id: String,
        agent: String,
        trigger: Trigger,
        started_at: String,
        ended_at: String,
        outcome: AgentRunOutcome,
        output: String,
    ) -> Self {
        Self {
            version: RECEIPT_VERSION,
            run_id,
            agent,
            trigger,
            started_at,
            ended_at,
            outcome,
            exit_code: None,
            output,
            diff: None,
            cost_usd: None,
            delivery_fingerprint: None,
            delivery_error: None,
            notify_error: None,
            feedback: None,
        }
    }

    pub fn skipped(run_id: String, agent: String, trigger: Trigger, now: String) -> Self {
        Self::new(
            run_id,
            agent,
            trigger,
            now.clone(),
            now,
            AgentRunOutcome::Skipped,
            String::new(),
        )
    }

    pub fn failed(
        run_id: String,
        agent: String,
        trigger: Trigger,
        started_at: String,
        ended_at: String,
        reason: String,
    ) -> Self {
        Self::new(
            run_id,
            agent,
            trigger,
            started_at,
            ended_at,
            AgentRunOutcome::Failed,
            reason,
        )
    }

    pub fn notify_summary<'a>(&'a self, hall: &'a str) -> NotifySummary<'a> {
        let tail = match self.output.char_indices().rev().nth(NOTIFY_TAIL_CHARS - 1) {
            Some((start, _)) => &self.output[start..],
            None => &self.output,
        };
        let tail_is_cut = tail.len() < self.output.len();
        NotifySummary {
            hall,
            run_id: &self.run_id,
            agent: &self.agent,
            trigger: self.trigger,
            outcome: self.outcome,
            exit_code: self.exit_code,
            started_at: &self.started_at,
            ended_at: &self.ended_at,
            tail,
            tail_is_cut,
        }
    }
}

/// Precedence: timeout, pending approval, the run's own declaration, exit code.
pub fn classify(
    exit_code: Option<i32>,
    timed_out: bool,
    blocked: bool,
    declared: Option<AgentRunOutcome>,
) -> AgentRunOutcome {
    match (timed_out, blocked, declared, exit_code) {
        (true, _, _, _) => AgentRunOutcome::TimedOut,
        (false, true, _, _) => AgentRunOutcome::Blocked,
        (false, false, Some(outcome), _) => outcome,
        (false, false, None, Some(0)) => AgentRunOutcome::Succeeded,
        _ => AgentRunOutcome::Failed,
    }
}

/// The outcome a prompt-driven run declares by ending its output with
/// `BLOCKED:` or `FAILED:` (case-sensitive, plain text, last non-empty line).
pub fn declared_outcome(output: &str) -> Option<AgentRunOutcome> {
    let last = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    if last.starts_with("BLOCKED:") {
        Some(AgentRunOutcome::Blocked)
    } else if last.starts_with("FAILED:") {
        Some(AgentRunOutcome::Failed)
    } else {
        None
    }
}

pub fn scrub(text: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|s| !s.is_empty())
        .fold(text.to_owned(), |acc, s| acc.replace(s, "[redacted]"))
}

pub fn run_id(started_at_rfc3339: &str, suffix: &str) -> String {
    let digits: String = started_at_rfc3339
        .chars()
        .filter(char::is_ascii_digit)
        .take(14)
        .collect();
    let (date, time) = digits.split_at(digits.len().min(8));
    format!("{date}T{time}Z-{suffix}")
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/receipt.rs"]
mod tests;
