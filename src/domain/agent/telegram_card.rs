//! The Telegram card a finished run sends: an HTML header, the output tail in
//! a collapsed quote, and a prompt that hands the run to an agent.

use super::receipt::{AgentRunOutcome, NotifySummary, TELEGRAM_TEXT_LIMIT, utf16_suffix};

pub const COPY_PROMPT_LABEL: &str = "📋 Copy investigation prompt";
/// Telegram's `CopyTextButton.text` accepts 1-256 characters.
const COPY_TEXT_LIMIT: usize = 256;

impl NotifySummary<'_> {
    pub fn telegram_html(&self) -> String {
        let icon = outcome_icon(self.outcome);
        let outcome = self.outcome.as_str();
        let meta = self.meta_line();
        let mut html = format!(
            "{icon} <b>{}</b> · {outcome}\n<i>{}</i>",
            escape(self.agent),
            escape(&meta)
        );
        let tail = self.readable_tail();
        if !tail.is_empty() {
            let visible_header = format!("{icon} {} · {outcome}\n{meta}\n\n", self.agent);
            let room = TELEGRAM_TEXT_LIMIT.saturating_sub(visible_header.encode_utf16().count());
            html.push_str("\n\n<blockquote expandable>");
            html.push_str(&markdown_html(utf16_suffix(&tail, room)));
            html.push_str("</blockquote>");
        }
        html
    }

    pub fn investigation_prompt(&self) -> String {
        let (hall, agent, run) = (self.hall, self.agent, self.run_id);
        format!(
            "In hall {hall}, investigate agent run {run} ({}): `ivar agent logs {agent} --run {run}`. \
             Check it against agents/{agent}.md with the hall skills; report only what needs action.",
            self.outcome.as_str()
        )
        .chars()
        .take(COPY_TEXT_LIMIT)
        .collect()
    }

    fn meta_line(&self) -> String {
        let details = self.run_details();
        format!(
            "{} · {} · {}{details}",
            self.hall,
            self.trigger.as_str(),
            self.run_id
        )
    }

    fn readable_tail(&self) -> String {
        let tail = self.tail.trim();
        if !self.tail_is_cut {
            return tail.to_owned();
        }
        match tail.split_once('\n') {
            Some((_, whole_lines)) if !whole_lines.trim().is_empty() => {
                format!("…\n{}", whole_lines.trim_start())
            }
            _ => format!("…{tail}"),
        }
    }
}

fn outcome_icon(outcome: AgentRunOutcome) -> &'static str {
    match outcome {
        AgentRunOutcome::Succeeded => "✅",
        AgentRunOutcome::Failed => "❌",
        AgentRunOutcome::Blocked => "⛔",
        AgentRunOutcome::Skipped => "⏭",
        AgentRunOutcome::TimedOut => "⏱",
    }
}

fn markdown_html(text: &str) -> String {
    text.lines().map(line_html).collect::<Vec<_>>().join("\n")
}

fn line_html(line: &str) -> String {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let hashes = body.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
        let heading = body[hashes..].trim().replace("**", "");
        return format!("{indent}<b>{}</b>", escape(&heading));
    }
    match body.strip_prefix("- ").or_else(|| body.strip_prefix("* ")) {
        Some(item) => format!("{indent}• {}", inline_html(item)),
        None => format!("{indent}{}", inline_html(body)),
    }
}

/// Bold never wraps code: Telegram refuses a `code` entity inside `bold`.
fn inline_html(text: &str) -> String {
    paired_spans(
        text,
        "`",
        |code| format!("<code>{}</code>", escape(code)),
        |plain| {
            paired_spans(
                plain,
                "**",
                |bold| format!("<b>{}</b>", escape(bold)),
                escape,
            )
        },
    )
}

fn paired_spans(
    text: &str,
    marker: &str,
    inside: impl Fn(&str) -> String,
    outside: impl Fn(&str) -> String,
) -> String {
    let parts: Vec<&str> = text.split(marker).collect();
    let last = parts.len() - 1;
    let mut html = String::new();
    for (index, part) in parts.iter().enumerate() {
        let opens_a_span = index % 2 == 1;
        let is_closed = index < last;
        if opens_a_span && is_closed && !part.is_empty() {
            html.push_str(&inside(part));
        } else {
            if opens_a_span {
                html.push_str(&outside(marker));
            }
            html.push_str(&outside(part));
            if opens_a_span && is_closed {
                html.push_str(&outside(marker));
            }
        }
    }
    html
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/telegram_card.rs"]
mod tests;
