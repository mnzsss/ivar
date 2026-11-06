//! `ivar memory lint`: the page rules checked against the files on disk.

use serde::Serialize;

use crate::action::memory::reindex::walk_pages;
use crate::action::{Ctx, discover_hall};
use crate::domain::memory_page::PageIssueCode;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::layout::Layout;
use crate::store::memory::anchors;
use crate::store::memory::page_source::{parse_page, validate_page};

#[derive(Debug, Clone, Serialize)]
pub struct LintIssue {
    pub path: String,
    pub code: &'static str,
    pub severity: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct LintOutcome {
    pub issues: Vec<LintIssue>,
}

impl LintOutcome {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|issue| issue.severity == "error")
    }
}

impl WriteHuman for LintOutcome {
    fn write_human(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        for issue in &self.issues {
            writeln!(w, "{}: {} {}", issue.path, issue.code, issue.message)?;
        }
        Ok(())
    }
}

const fn lint_code(issue: PageIssueCode) -> &'static str {
    match issue {
        PageIssueCode::KindUnknown => "lint.kind_unknown",
        PageIssueCode::KindMismatch
        | PageIssueCode::FrontmatterInvalid
        | PageIssueCode::ExpiresAtInvalid => "lint.frontmatter_invalid",
        PageIssueCode::Misplaced => "lint.misplaced",
        PageIssueCode::FieldUnknown => "lint.field_unknown",
    }
}

fn error(rel: &str, code: &'static str, message: impl Into<String>) -> LintIssue {
    LintIssue {
        path: rel.to_owned(),
        code,
        severity: "error",
        message: message.into(),
    }
}

#[must_use]
pub fn lint_in(layout: &Layout) -> Vec<LintIssue> {
    let mut found = Vec::new();
    for file in walk_pages(layout) {
        let Ok(source) = std::fs::read_to_string(&file.path) else {
            continue;
        };
        for issue in validate_page(&file.rel, &source) {
            found.push(error(&file.rel, lint_code(issue.code), issue.message));
        }
        if let Ok(page) = parse_page(&file.rel, &source) {
            found.extend(broken_links(layout, &file.rel, &page.body));
        }
    }
    found.extend(broken_anchor_warnings(layout));
    found
}

/// # Errors
///
/// Fails when no hall is found from the working directory.
pub fn lint(ctx: &Ctx) -> Outcome<LintOutcome> {
    let layout = discover_hall(ctx)?;
    Ok(Report::new(LintOutcome {
        issues: lint_in(&layout),
    }))
}

fn broken_anchor_warnings(layout: &Layout) -> Vec<LintIssue> {
    if !layout.graph_db().exists() || !layout.memory_db().exists() {
        return Vec::new();
    }
    let _ = crate::action::memory::reindex::refresh_if_free(layout);
    let Ok(conn) = crate::store::memory::db::open_read_only(layout.memory_db().as_std_path())
    else {
        return Vec::new();
    };
    if !anchors::attach_graph(&conn, &layout.graph_db()) {
        return Vec::new();
    }
    let broken = anchors::broken_anchors(&conn).unwrap_or_default();
    anchors::detach_graph(&conn);
    broken
        .into_iter()
        .map(|(page, anchor)| LintIssue {
            path: page,
            code: "lint.anchor_broken",
            severity: "warning",
            message: match anchor.symbol {
                Some(symbol) => format!(
                    "cites `{symbol}` in {}:{}, which the graph no longer has",
                    anchor.repo, anchor.path
                ),
                None => format!(
                    "cites {}:{}, which the graph no longer has",
                    anchor.repo, anchor.path
                ),
            },
        })
        .collect()
}

fn broken_links(layout: &Layout, rel: &str, body: &str) -> Vec<LintIssue> {
    markdown_targets(body)
        .into_iter()
        .filter_map(|target| resolve_relative(rel, &target))
        .filter(|resolved| resolved.starts_with("docs/memory/") && resolved.ends_with(".md"))
        .filter(|resolved| !layout.root().join(resolved).exists())
        .map(|resolved| {
            error(
                rel,
                "lint.link_broken",
                format!("links to {resolved}, which does not exist"),
            )
        })
        .collect()
}

fn markdown_targets(body: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut rest = line;
        while let Some((_, after)) = rest.split_once("](") {
            let Some((destination, tail)) = after.split_once(')') else {
                break;
            };
            let target = destination.split('#').next().unwrap_or("").trim();
            if !target.is_empty() && !target.contains("://") {
                targets.push(target.to_owned());
            }
            rest = tail;
        }
    }
    targets
}

fn resolve_relative(page_rel: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = page_rel.split('/').collect();
    parts.pop();
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/lint.rs"]
mod tests;
