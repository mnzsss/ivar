use std::collections::HashMap;

use super::ContentHitInfo;
use crate::action::graph::query::types::QueryError;
use crate::store::graph::db::GraphDb;

/// File-content FTS hits for each term, keyed by `(repo, path)`, first hit wins.
pub(super) fn search_content_hits(
    db: &GraphDb,
    terms: &[String],
    repo: Option<&str>,
) -> Result<HashMap<(String, String), ContentHitInfo>, QueryError> {
    let mut hits = HashMap::new();
    for term in terms {
        if term.trim().is_empty() {
            continue;
        }
        let term_hits = db.search_file_content(term, repo, 50)?;
        for hit in term_hits {
            let key = (hit.repo.clone(), hit.path.clone());
            let (start_line, excerpt) = find_line_and_excerpt(&hit.indexed_content, term);
            hits.entry(key).or_insert(ContentHitInfo {
                repo: hit.repo,
                path: hit.path,
                rank: hit.rank,
                start_line,
                excerpt,
                content_truncated: hit.content_truncated,
            });
        }
    }
    Ok(hits)
}

pub(super) fn find_line_and_excerpt(content: &str, term: &str) -> (usize, String) {
    let term_lower = term.to_ascii_lowercase();
    for (idx, line) in content.lines().enumerate() {
        if line.to_ascii_lowercase().contains(&term_lower) {
            return (idx + 1, line.trim().to_owned());
        }
    }
    let first_line = content.lines().next().unwrap_or("").trim().to_owned();
    (1, first_line)
}
