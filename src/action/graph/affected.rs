//! Reverse dependency test finder for affected test execution.
//!
//! Synchronously walks reverse import and call dependencies starting from changed
//! source files up to `max_depth` hops, isolating and filtering reachable test files.

use std::collections::HashSet;
use std::io::BufRead;

use rusqlite::params;
use thiserror::Error;

use crate::domain::graph::AffectedResult;
use crate::store::graph::db::GraphDb;

/// Error returned during affected test resolution.
#[derive(Debug, Error)]
pub enum AffectedError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("invalid depth: {0} (must be > 0)")]
    InvalidDepth(usize),
}

/// Helper function to parse file paths line-by-line from a buffered reader.
///
/// Trims whitespace and skips empty lines or comment lines starting with `#`.
pub fn parse_files_from_reader<R: BufRead>(reader: R) -> Vec<String> {
    reader
        .lines()
        .map_while(Result::ok)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// Checks if a file path matches standard test file patterns.
///
/// Supported heuristics:
/// - `tests/**` or `__tests__/**`
/// - `*_test.rs`, `*test*.rs`
/// - `*.test.ts`, `*.test.tsx`, `*.test.js`
/// - `*.spec.ts`, `*.spec.tsx`, `*.spec.js`
pub fn is_test_file(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();

    // Check directory prefix/components
    if parts
        .iter()
        .any(|&p| p == "tests" || p == "__tests__" || p == "test")
    {
        return true;
    }

    if let Some(&filename) = parts.last() {
        if filename.ends_with(".test.ts")
            || filename.ends_with(".test.tsx")
            || filename.ends_with(".test.js")
            || filename.ends_with(".test.jsx")
            || filename.ends_with(".spec.ts")
            || filename.ends_with(".spec.tsx")
            || filename.ends_with(".spec.js")
            || filename.ends_with(".spec.jsx")
        {
            return true;
        }

        if let Some(stem) = filename.strip_suffix(".rs")
            && (stem == "test"
                || stem.ends_with("_test")
                || stem.starts_with("test_")
                || stem.contains("test"))
        {
            return true;
        }
    }

    false
}

/// Finds all test files affected by changes in the given source files.
///
/// Walks reverse dependencies (incoming `IMPORTS`, `CALLS`, and `CROSS_IMPORTS` edges)
/// across symbols and files up to `max_depth` hops.
pub fn find_affected_tests(
    db: &GraphDb,
    changed_files: &[String],
    repo: Option<&str>,
    max_depth: usize,
) -> Result<AffectedResult, AffectedError> {
    if max_depth == 0 {
        return Err(AffectedError::InvalidDepth(0));
    }

    if changed_files.is_empty() {
        return Ok(AffectedResult {
            changed_files: Vec::new(),
            affected_test_files: Vec::new(),
        });
    }

    let conn = db.conn();

    // Normalize input paths
    let normalized_changed: Vec<String> = changed_files
        .iter()
        .map(|f| f.trim().replace('\\', "/"))
        .filter(|f| !f.is_empty())
        .collect();

    if normalized_changed.is_empty() {
        return Ok(AffectedResult {
            changed_files: Vec::new(),
            affected_test_files: Vec::new(),
        });
    }

    // Step 1: Find file IDs for all changed files
    let mut file_ids = Vec::new();
    let mut initial_test_files = Vec::new();

    for path in &normalized_changed {
        let mut stmt = conn.prepare_cached(
            "SELECT id, path FROM files WHERE (?1 IS NULL OR repo = ?1) AND (path = ?2 OR path LIKE ?3)",
        )?;
        let like_pattern = format!("%/{}", path);
        let mut rows = stmt.query(params![repo, path, like_pattern])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let file_path: String = row.get(1)?;
            file_ids.push(id);
            if is_test_file(&file_path) {
                initial_test_files.push(file_path);
            }
        }
    }

    let mut affected_tests_set: HashSet<String> = initial_test_files.into_iter().collect();

    if !file_ids.is_empty() {
        // Step 2: Use recursive CTE to traverse reverse dependencies
        // We start with all symbols belonging to the changed files,
        // and find caller/importer symbols or referencing edges.
        // Also find direct file-level edges where from_symbol is in another file.
        let sql = "
            WITH RECURSIVE reverse_deps(file_id, depth, visited_files) AS (
                -- Seed with changed files
                SELECT f.id, 0, ',' || CAST(f.id AS TEXT) || ','
                FROM files f
                WHERE f.id = ?1

                UNION

                -- Transitive reverse dependencies:
                -- Any edge from file A to file B (where B is in reverse_deps)
                -- e.to_symbol_id belongs to B, or e.to_name matches a symbol in B
                SELECT DISTINCT
                    e.file_id,
                    rd.depth + 1,
                    rd.visited_files || CAST(e.file_id AS TEXT) || ','
                FROM reverse_deps rd
                JOIN symbols s ON s.file_id = rd.file_id
                JOIN edges e ON (
                    e.to_symbol_id = s.id
                    OR (e.to_symbol_id IS NULL AND e.to_name = s.name)
                )
                WHERE rd.depth < ?2
                  AND e.file_id IS NOT NULL
                  AND instr(rd.visited_files, ',' || CAST(e.file_id AS TEXT) || ',') = 0
                  AND (?3 IS NULL OR e.repo = ?3)
            )
            SELECT DISTINCT f.path
            FROM reverse_deps rd
            JOIN files f ON rd.file_id = f.id
            WHERE rd.depth > 0
        ";

        for fid in file_ids {
            let mut stmt = conn.prepare_cached(sql)?;
            let mut rows = stmt.query(params![fid, max_depth as i64, repo])?;
            while let Some(row) = rows.next()? {
                let path: String = row.get(0)?;
                if is_test_file(&path) {
                    affected_tests_set.insert(path);
                }
            }
        }
    }

    let mut affected_test_files: Vec<String> = affected_tests_set.into_iter().collect();
    affected_test_files.sort();

    Ok(AffectedResult {
        changed_files: normalized_changed,
        affected_test_files,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/affected.rs"]
mod tests;
