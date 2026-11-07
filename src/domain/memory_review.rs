//! A feature's memory review, computed from its Observations: the pages it
//! wrote, the gaps, and the durable pages its diff may have made stale.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::memory_page::PageKind;

const MEMORY_DIR: &str = "docs/memory/";
const WRITE_IMPORTANCE: u8 = 4;
const FAILURE_IMPORTANCE: u8 = 6;
/// The `source_event` of a fact built from a `memory_usage` row: a query the
/// MCP server or the CLI answered, which no provider hook captured.
pub const USAGE_EVENT: &str = "memory.usage";
const WRITE_KEYS: [&str; 3] = ["file_path", "filePath", "path"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationFact {
    pub source_event: String,
    pub occurred_at: String,
    pub importance: u8,
    pub body: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageAction {
    Written,
    Patched,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteSource {
    Tool,
    Capture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrittenPage {
    pub path: String,
    pub kind: String,
    pub action: PageAction,
    pub source: WriteSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    Gotcha,
    Update,
    AnalysisWithoutQuery,
}

impl GapKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gotcha => "gotcha",
            Self::Update => "update",
            Self::AnalysisWithoutQuery => "analysis_without_query",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub kind: GapKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum StaleReason {
    ChangedFile {
        repo: String,
        path: String,
    },
    BrokenAnchor {
        repo: String,
        path: String,
        symbol: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StalePage {
    pub path: String,
    pub reasons: Vec<StaleReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Updated,
    StillValid,
}

fn page_target(rel: &str) -> Option<(String, String)> {
    let (kind, file) = rel.strip_prefix(MEMORY_DIR)?.split_once('/')?;
    let kind = PageKind::parse(kind).filter(|kind| kind.is_durable())?;
    (!file.contains('/') && file.ends_with(".md"))
        .then(|| (rel.to_owned(), kind.as_str().to_owned()))
}

#[must_use]
pub fn is_durable_page(rel: &str) -> bool {
    page_target(rel).is_some()
}

fn hall_relative<'a>(path: &'a str, hall_root: &str) -> Option<&'a str> {
    if std::path::Path::new(path).is_relative() {
        return Some(path);
    }
    path.strip_prefix(hall_root.trim_end_matches('/'))?
        .strip_prefix('/')
}

#[must_use]
pub fn memory_page_target(tool_input: &Value, hall_root: &str) -> Option<(String, String)> {
    let path = WRITE_KEYS
        .iter()
        .find_map(|key| tool_input.get(*key)?.as_str())?;
    page_target(hall_relative(path, hall_root)?)
}

/// A body cut by the sanitize limit is no longer JSON; the page path is
/// still read when it survived the cut.
fn captured_target(body: &Value, hall_root: &str) -> Option<(String, String)> {
    if let Value::String(raw) = body {
        let absolute = format!("{}/{MEMORY_DIR}", hall_root.trim_end_matches('/'));
        let relative = format!("\":\"{MEMORY_DIR}");
        let start = [absolute, relative].iter().find_map(|prefix| {
            Some(raw.find(prefix.as_str())? + prefix.len() - MEMORY_DIR.len())
        })?;
        let rel: String = raw
            .get(start..)?
            .chars()
            .take_while(|c| !matches!(c, '"' | '\\' | '\'' | ' ' | '\n'))
            .collect();
        return page_target(&rel);
    }
    memory_page_target(body.get("input")?, hall_root)
}

fn tool_write(body: &Value) -> Option<WrittenPage> {
    let action = match body.get("action")?.as_str()? {
        "written" => PageAction::Written,
        "patched" => PageAction::Patched,
        "deleted" => PageAction::Deleted,
        _ => return None,
    };
    Some(WrittenPage {
        path: body.get("path")?.as_str()?.to_owned(),
        kind: body.get("kind")?.as_str()?.to_owned(),
        action,
        source: WriteSource::Tool,
    })
}

#[must_use]
pub fn written_pages(facts: &[ObservationFact], hall_root: &str) -> Vec<WrittenPage> {
    let mut pages: BTreeMap<String, WrittenPage> = BTreeMap::new();
    for fact in facts {
        let page = match fact.source_event.as_str() {
            "memory.page_written" => tool_write(&fact.body),
            "session.tool" if fact.importance == WRITE_IMPORTANCE => {
                captured_target(&fact.body, hall_root).map(|(path, kind)| WrittenPage {
                    path,
                    kind,
                    action: PageAction::Written,
                    source: WriteSource::Capture,
                })
            }
            _ => None,
        };
        let Some(page) = page else { continue };
        pages.insert(page.path.clone(), page);
    }
    pages.into_values().collect()
}

fn command(body: &Value) -> Option<&str> {
    body.get("input")?.get("command")?.as_str()
}

fn is_memory_query(body: &Value) -> bool {
    body.get("tool")
        .and_then(Value::as_str)
        .is_some_and(|tool| tool.ends_with("memory_query"))
        || command(body).is_some_and(|command| command.contains("ivar memory query"))
}

fn queried_before_approval(facts: &[ObservationFact], feature: &str) -> bool {
    let approval = format!("ivar plan approve {feature} analysis");
    let tools = || {
        facts
            .iter()
            .filter(|fact| fact.source_event == "session.tool")
    };
    let approved_at = tools()
        .find(|fact| command(&fact.body).is_some_and(|command| command.contains(&approval)))
        .map(|fact| fact.occurred_at.as_str());
    let captured = tools().filter(|fact| is_memory_query(&fact.body));
    let served = facts.iter().filter(|fact| fact.source_event == USAGE_EVENT);
    captured
        .chain(served)
        .any(|fact| approved_at.is_none_or(|at| fact.occurred_at.as_str() < at))
}

#[must_use]
pub fn gaps(
    facts: &[ObservationFact],
    written: &[WrittenPage],
    feature: &str,
    analysis_approved: bool,
) -> Vec<Gap> {
    let kept = |kind: &str| {
        written
            .iter()
            .any(|page| page.kind == kind && page.action != PageAction::Deleted)
    };
    let failures = facts
        .iter()
        .filter(|fact| fact.source_event == "session.tool" && fact.importance >= FAILURE_IMPORTANCE)
        .count();
    let mut gaps = Vec::new();
    if failures > 0 && !kept("gotcha") {
        gaps.push(Gap {
            kind: GapKind::Gotcha,
            detail: format!(
                "{failures} tool call(s) failed and no gotcha page was written in `{feature}`"
            ),
        });
    }
    if !kept("update") {
        gaps.push(Gap {
            kind: GapKind::Update,
            detail: format!("no update page was written in `{feature}`"),
        });
    }
    if analysis_approved && !queried_before_approval(facts, feature) {
        gaps.push(Gap {
            kind: GapKind::AnalysisWithoutQuery,
            detail: format!(
                "the analysis of `{feature}` was approved without a memory_query in its sessions"
            ),
        });
    }
    gaps
}

#[must_use]
pub fn stale_pages(
    changed: &[(String, String, String)],
    broken: &[(String, String, String, Option<String>)],
    written: &[WrittenPage],
) -> Vec<StalePage> {
    let mut pages: BTreeMap<&str, Vec<StaleReason>> = BTreeMap::new();
    for (page, repo, path) in changed {
        pages
            .entry(page.as_str())
            .or_default()
            .push(StaleReason::ChangedFile {
                repo: repo.clone(),
                path: path.clone(),
            });
    }
    for (page, repo, path, symbol) in broken {
        pages
            .entry(page.as_str())
            .or_default()
            .push(StaleReason::BrokenAnchor {
                repo: repo.clone(),
                path: path.clone(),
                symbol: symbol.clone(),
            });
    }
    pages
        .into_iter()
        .filter(|(path, _)| !written.iter().any(|page| page.path == *path))
        .map(|(path, reasons)| StalePage {
            path: path.to_owned(),
            reasons,
        })
        .collect()
}

#[must_use]
pub fn missing_verdicts(stale: &[StalePage], verdicts: &[(String, Verdict)]) -> Vec<String> {
    stale
        .iter()
        .filter(|page| !verdicts.iter().any(|(path, _)| *path == page.path))
        .map(|page| page.path.clone())
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/domain/memory_review.rs"]
mod tests;
