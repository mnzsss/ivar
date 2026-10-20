//! What one scheduled-agent run records, and the rules that classify it.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub const RECEIPT_VERSION: u32 = 1;

const NOTIFY_TAIL_CHARS: usize = 2000;

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
    pub run_id: &'a str,
    pub agent: &'a str,
    pub trigger: Trigger,
    pub outcome: AgentRunOutcome,
    pub exit_code: Option<i32>,
    pub started_at: &'a str,
    pub ended_at: &'a str,
    pub tail: &'a str,
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

    pub fn notify_summary(&self) -> NotifySummary<'_> {
        let tail = match self.output.char_indices().rev().nth(NOTIFY_TAIL_CHARS - 1) {
            Some((start, _)) => &self.output[start..],
            None => &self.output,
        };
        NotifySummary {
            run_id: &self.run_id,
            agent: &self.agent,
            trigger: self.trigger,
            outcome: self.outcome,
            exit_code: self.exit_code,
            started_at: &self.started_at,
            ended_at: &self.ended_at,
            tail,
        }
    }
}

pub fn classify(exit_code: Option<i32>, timed_out: bool, blocked: bool) -> AgentRunOutcome {
    match (timed_out, blocked, exit_code) {
        (true, _, _) => AgentRunOutcome::TimedOut,
        (false, true, _) => AgentRunOutcome::Blocked,
        (false, false, Some(0)) => AgentRunOutcome::Succeeded,
        _ => AgentRunOutcome::Failed,
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
