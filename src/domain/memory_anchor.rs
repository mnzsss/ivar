//! Anchor candidates: code references found in a memory page body.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::memory_page::PageKind;

pub const MEMORY_DOCS_PREFIX: &str = "docs/memory/";
pub const MEMORY_EPHEMERAL_PREFIX: &str = ".ivar/memory/";

const MIN_SPAN_CHARS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    Path { repo: Option<String>, path: String },
    Symbol { scope: Option<String>, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Anchor {
    pub repo: String,
    pub path: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnchorView {
    pub repo: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub broken: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelatedPage {
    pub path: String,
    pub kind: String,
    pub title: String,
    pub anchors: usize,
}

#[must_use]
pub fn extract_candidates(body: &str) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for raw in code_spans(line).into_iter().chain(link_targets(line)) {
            if let Some(candidate) = classify(raw)
                && !found.contains(&candidate)
            {
                found.push(candidate);
            }
        }
    }
    found
}

fn code_spans(line: &str) -> Vec<&str> {
    let mut parts = line.split('`').skip(1).step_by(2).collect::<Vec<_>>();
    if line.matches('`').count() % 2 == 1 {
        parts.pop();
    }
    parts
}

fn link_targets(line: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rem = line;
    while let Some(start) = rem.find("](") {
        rem = &rem[start + 2..];
        let Some(end) = rem.find(')') else { break };
        targets.push(&rem[..end]);
        rem = &rem[end + 1..];
    }
    targets
}

fn classify(raw: &str) -> Option<Candidate> {
    let raw = raw.trim();
    if raw.chars().count() < MIN_SPAN_CHARS
        || raw.chars().any(char::is_whitespace)
        || raw.contains("://")
    {
        return None;
    }
    let raw = raw.split('#').next()?;
    let raw = strip_line_suffix(raw);
    if raw.contains('/') || has_file_extension(raw) {
        return classify_path(raw);
    }
    classify_symbol(raw.strip_suffix("()").unwrap_or(raw))
}

fn strip_line_suffix(raw: &str) -> &str {
    match raw.rsplit_once(':') {
        Some((head, tail))
            if !tail.is_empty()
                && tail.chars().all(|c| c.is_ascii_digit() || c == '-')
                && tail.starts_with(|c: char| c.is_ascii_digit()) =>
        {
            head
        }
        _ => raw,
    }
}

fn has_file_extension(raw: &str) -> bool {
    raw.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && !ext.starts_with(|c: char| c.is_ascii_digit())
    })
}

fn classify_path(raw: &str) -> Option<Candidate> {
    if raw.ends_with('/') {
        return None;
    }
    let trimmed = raw.trim_start_matches("./").trim_start_matches("../");
    if is_memory_page_target(trimmed) {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix(".ivar/repos/") {
        let mut parts = rest.splitn(3, '/');
        let (repo, _branch, path) = (parts.next()?, parts.next()?, parts.next()?);
        return Some(Candidate::Path {
            repo: Some(repo.to_owned()),
            path: path.to_owned(),
        });
    }
    if let Some((repo, path)) = trimmed.split_once(':')
        && !repo.contains('/')
        && is_repo_name(repo)
    {
        return Some(Candidate::Path {
            repo: Some(repo.to_owned()),
            path: path.to_owned(),
        });
    }
    Some(Candidate::Path {
        repo: None,
        path: trimmed.to_owned(),
    })
}

fn classify_symbol(raw: &str) -> Option<Candidate> {
    let (scope, name) = match raw.rsplit_once("::") {
        Some((scope, name)) => (Some(scope.rsplit("::").next()?.to_owned()), name),
        None => (None, raw),
    };
    (is_identifier(name) && scope.as_deref().is_none_or(is_identifier)).then(|| Candidate::Symbol {
        scope,
        name: name.to_owned(),
    })
}

fn is_memory_page_target(path: &str) -> bool {
    if path.starts_with(MEMORY_DOCS_PREFIX) || path.starts_with(MEMORY_EPHEMERAL_PREFIX) {
        return true;
    }
    let first = path.split('/').next().unwrap_or_default();
    path.ends_with(".md") && PageKind::parse(first).is_some()
}

fn is_repo_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "../../tests/unit/domain/memory_anchor.rs"]
mod tests;
