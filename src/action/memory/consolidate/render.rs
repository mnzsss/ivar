//! The Markdown of a `session` page, rendered from one conversation's observations.

use std::fmt::Write as _;

use serde_json::Value;

use crate::action::memory::mcp::pages::render_meta;
use crate::domain::memory_page::PageMeta;
use crate::store::memory::conversations::{ConversationHead, ConversationRow};

pub const TITLE_CHARS: usize = 80;
const LINE_CHARS: usize = 200;
const MAX_FILES: usize = 30;
const MAX_FAILURES: usize = 10;
const MAX_HIGHLIGHTS: usize = 10;

#[derive(Debug)]
pub struct Conversation<'a> {
    pub head: &'a ConversationHead,
    pub rows: &'a [ConversationRow],
    pub attached: &'a [ConversationRow],
}

/// The page, or `None` when no row reaches importance 4.
#[must_use]
pub fn render(conversation: &Conversation<'_>, preserved: &PageMeta) -> Option<String> {
    let mut all = conversation.rows.iter().chain(conversation.attached);
    if !all.any(|row| row.importance >= 4) {
        return None;
    }
    let title = title(conversation);
    let meta = PageMeta {
        pinned: preserved.pinned,
        expires_at: preserved.expires_at.clone(),
        ..PageMeta::default()
    };
    Some(render_meta(
        Some(&title),
        &meta,
        &body(conversation, &title),
    ))
}

fn title(conversation: &Conversation<'_>) -> String {
    conversation
        .rows
        .iter()
        .find(|row| row.source_event == "session.prompted" && row.origin == "user")
        .and_then(|row| first_line(&row.body))
        .map(|line| line.chars().take(TITLE_CHARS).collect())
        .unwrap_or_else(|| {
            let provider = conversation
                .rows
                .first()
                .map_or("", |row| row.provider.as_str());
            format!(
                "Session {provider} {}",
                conversation.head.first_at.get(..10).unwrap_or("")
            )
        })
}

fn body(conversation: &Conversation<'_>, title: &str) -> String {
    let head = conversation.head;
    let rows = conversation.rows;
    let field = |pick: fn(&ConversationRow) -> &str| {
        rows.iter()
            .map(pick)
            .find(|value| !value.is_empty())
            .unwrap_or("—")
    };
    let mut out = String::new();
    let _ = writeln!(out, "# {title}\n");
    let _ = writeln!(
        out,
        "- Provider: {} · Feature: {} · Repo: {}",
        field(|row| &row.provider),
        field(|row| &row.feature),
        field(|row| &row.repo)
    );
    let session_id = if head.session_id.is_empty() {
        "—"
    } else {
        &head.session_id
    };
    let _ = writeln!(
        out,
        "- Session: `{}` · ivar session: `{session_id}`",
        head.native_session_id
    );
    let _ = writeln!(out, "- Window: {} → {}", head.first_at, head.last_at);

    let mut files: Vec<String> = Vec::new();
    for path in tool_rows(rows, 4).filter_map(|tool| written_path(&tool)) {
        if files.len() < MAX_FILES && !files.contains(&path) {
            files.push(path);
        }
    }
    section(
        &mut out,
        "Files written",
        files.iter().map(|path| format!("`{path}`")),
    );

    let failures = tool_rows(rows, 6)
        .filter_map(|tool| failure(&tool))
        .take(MAX_FAILURES);
    section(&mut out, "Failures", failures);

    let delivery = conversation.attached.iter().map(|row| {
        format!(
            "{} {} {}",
            row.occurred_at,
            row.source_event,
            clip(&row.body)
        )
    });
    section(&mut out, "Delivery", delivery);

    let highlights = rows
        .iter()
        .filter(|row| row.importance >= 5)
        .take(MAX_HIGHLIGHTS)
        .map(|row| format!("{}: {}", row.source_event, clip(&row.body)));
    section(&mut out, "Highlights", highlights);
    out
}

fn section(out: &mut String, heading: &str, items: impl Iterator<Item = String>) {
    let mut items = items.peekable();
    if items.peek().is_none() {
        return;
    }
    let _ = writeln!(out, "\n## {heading}\n");
    for item in items {
        let _ = writeln!(out, "- {item}");
    }
}

fn tool_rows(rows: &[ConversationRow], importance: u8) -> impl Iterator<Item = Value> + '_ {
    rows.iter()
        .filter(move |row| row.source_event == "session.tool" && row.importance == importance)
        .filter_map(|row| serde_json::from_str(&row.body).ok())
}

fn written_path(tool: &Value) -> Option<String> {
    let input = tool.get("input")?;
    ["file_path", "filePath", "path"]
        .iter()
        .find_map(|key| input.get(key)?.as_str())
        .map(str::to_owned)
}

fn failure(tool: &Value) -> Option<String> {
    let name = tool.get("tool")?.as_str()?;
    let result = tool.get("result").and_then(Value::as_str).unwrap_or("");
    Some(format!("{name}: {}", clip(result)))
}

fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn clip(text: &str) -> String {
    first_line(text)
        .unwrap_or("")
        .chars()
        .take(LINE_CHARS)
        .collect()
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/consolidate_render.rs"]
mod tests;
