use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    #[default]
    Bug,
    Proposal,
}

impl FeedbackKind {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Proposal => "proposal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackStatus {
    #[default]
    Open,
    Published,
    #[serde(other)]
    Unknown,
}

impl FeedbackStatus {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Published => "published",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub title: String,
    pub kind: FeedbackKind,
    pub status: FeedbackStatus,
    pub created_at: String,
    pub ivar_version: String,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub id: String,
    pub frontmatter: Frontmatter,
    pub body: String,
}

impl FeedbackEntry {
    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.frontmatter.status != FeedbackStatus::Unknown
    }
}

#[must_use]
pub fn slug(title: &str) -> String {
    let mut result = String::with_capacity(title.len());
    let mut last_was_dash = true;

    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            result.push('-');
            last_was_dash = true;
        }
    }

    while result.ends_with('-') {
        result.pop();
    }

    if result.len() > 48 {
        result.truncate(48);
        while result.ends_with('-') {
            result.pop();
        }
    }

    if result.is_empty() {
        "entry".to_owned()
    } else {
        result
    }
}

#[must_use]
pub fn entry_id(seq: u32, title: &str) -> String {
    format!("{seq:03}-{}", slug(title))
}

pub fn next_seq<'a>(ids: impl IntoIterator<Item = &'a str>) -> u32 {
    let mut max_seq = 0;
    for id in ids {
        if let Some(prefix) = id.split('-').next()
            && let Ok(seq) = prefix.parse::<u32>()
            && seq > max_seq
        {
            max_seq = seq;
        }
    }
    max_seq + 1
}

#[derive(Debug, Clone, Default)]
pub struct Redactions {
    pub hall: Option<String>,
    pub home: Option<String>,
    pub user: Option<String>,
    pub host: Option<String>,
}

#[must_use]
pub fn redact(text: &str, r: &Redactions) -> String {
    let mut result = text.to_owned();

    // 1. Hall root path -> "<hall>"
    if let Some(hall) = &r.hall
        && hall.len() >= 2
    {
        result = result.replace(hall, "<hall>");
    }

    // 2. Home path -> "~"
    if let Some(home) = &r.home
        && home.len() >= 2
    {
        result = result.replace(home, "~");
    }

    // 3. User -> "<user>" (whole word)
    if let Some(user) = &r.user
        && user.len() >= 2
    {
        result = replace_whole_word(&result, user, "<user>");
    }

    // 4. Host -> "<host>" (whole word)
    if let Some(host) = &r.host
        && host.len() >= 2
    {
        result = replace_whole_word(&result, host, "<host>");
    }

    result
}

fn replace_whole_word(text: &str, target: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(pos) = text.get(cursor..).and_then(|sub| sub.find(target)) {
        let abs_pos = cursor + pos;
        let before_ok = abs_pos == 0
            || text
                .get(..abs_pos)
                .and_then(|s| s.chars().next_back())
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let after_idx = abs_pos + target.len();
        let after_ok = after_idx == text.len()
            || text
                .get(after_idx..)
                .and_then(|s| s.chars().next())
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');

        if before_ok && after_ok {
            if let Some(prefix) = text.get(cursor..abs_pos) {
                out.push_str(prefix);
            }
            out.push_str(replacement);
            cursor = after_idx;
        } else {
            let next_cursor = abs_pos + target.len();
            if let Some(chunk) = text.get(cursor..next_cursor) {
                out.push_str(chunk);
            }
            cursor = next_cursor;
        }
    }
    if let Some(rest) = text.get(cursor..) {
        out.push_str(rest);
    }
    out
}

#[must_use]
pub fn issue_title(entry: &FeedbackEntry) -> String {
    entry.frontmatter.title.clone()
}

#[must_use]
pub fn issue_body(entry: &FeedbackEntry) -> String {
    let mut body = String::new();
    body.push_str("### Environment\n");
    let _ = writeln!(body, "- ivar version: {}", entry.frontmatter.ivar_version);
    let _ = writeln!(
        body,
        "- OS/Arch: {}/{}",
        entry.frontmatter.os, entry.frontmatter.arch
    );
    if let Some(provider) = &entry.frontmatter.provider {
        let _ = writeln!(body, "- Provider: {provider}");
    }
    let _ = writeln!(body, "- Kind: {}\n", entry.frontmatter.kind.as_str());
    body.push_str(&entry.body);
    body
}

#[cfg(test)]
#[path = "../../tests/unit/domain/feedback.rs"]
mod tests;
