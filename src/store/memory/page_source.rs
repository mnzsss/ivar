use std::collections::BTreeMap;

use crate::domain::memory_page::{Page, PageIssue, PageMeta, ParsedFrontmatter, build_page};
use crate::infra::frontmatter;

const KNOWN_KEYS: [&str; 6] = ["title", "kind", "tags", "pinned", "relations", "expires_at"];

fn unknown_keys(source: &str) -> Vec<String> {
    frontmatter::parse::<BTreeMap<String, serde::de::IgnoredAny>>(source)
        .map(|keys| {
            keys.into_keys()
                .filter(|key| !KNOWN_KEYS.contains(&key.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

/// # Errors
///
/// Returns the issues that make the page unreadable.
pub fn parse_page(rel_path: &str, source: &str) -> Result<Page, Vec<PageIssue>> {
    let split_res = frontmatter::split(source);
    let (body, front_res) = match split_res {
        Ok(split) => {
            let body = split.body;
            let front = match frontmatter::parse::<PageMeta>(source) {
                Ok(meta) => Ok(ParsedFrontmatter {
                    meta,
                    unknown_fields: unknown_keys(source),
                }),
                Err(err) => Err(err.to_string()),
            };
            (body, front)
        }
        Err(err) => (source, Err(err.to_string())),
    };

    build_page(rel_path, front_res, body)
}

pub fn validate_page(rel_path: &str, source: &str) -> Vec<PageIssue> {
    match parse_page(rel_path, source) {
        Ok(page) => page.warnings(),
        Err(issues) => issues,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/store/memory/page_source.rs"]
mod tests;
