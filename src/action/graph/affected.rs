//! Reverse dependency test finder for affected test execution.
//!
//! Synchronously walks reverse import and call dependencies starting from changed
//! source files up to `max_depth` hops, isolating and filtering reachable test files,
//! retaining causal edges, selecting deterministic best paths, and deriving focused test commands.

use std::collections::{BTreeMap, HashSet};
use std::io::BufRead;
use std::path::Path;

use rusqlite::params;
use thiserror::Error;

use crate::domain::graph::{
    AffectedRecommendation, AffectedResult, CausalStep, EdgeKind, Provenance,
};
use crate::store::graph::db::{GraphDb, parse_edge_kind, parse_provenance};

/// Error returned during affected test resolution.
#[derive(Debug, Error)]
pub enum AffectedError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("invalid depth: {0} (must be > 0)")]
    InvalidDepth(usize),
}

/// Parses a list of file paths from an input reader (e.g. stdin or file).
///
/// Trims whitespace, strips empty lines and `#` comments.
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
        .any(|p| *p == "tests" || *p == "__tests__" || *p == "test")
    {
        return true;
    }

    // Check filename patterns
    if let Some(filename) = parts.last()
        && (filename.ends_with("_test.rs")
            || filename.ends_with(".test.ts")
            || filename.ends_with(".test.tsx")
            || filename.ends_with(".test.js")
            || filename.ends_with(".spec.ts")
            || filename.ends_with(".spec.tsx")
            || filename.ends_with(".spec.js")
            || filename.starts_with("test_"))
    {
        return true;
    }

    false
}

#[derive(Debug, Clone)]
struct TargetFileInfo {
    id: i64,
    repo: String,
    path: String,
}

#[derive(Debug, Clone)]
struct RawCausalEdge {
    from_file_id: i64,
    from_repo: String,
    from_path: String,
    from_symbol_name: Option<String>,
    _to_file_id: i64,
    _to_repo: String,
    to_path: String,
    to_symbol_name: Option<String>,
    edge_kind: EdgeKind,
    provenance: Provenance,
    confidence: f64,
    line: usize,
}

/// Derives an executable focused test command if the repository configuration and file layout prove it.
pub fn derive_test_command(
    repo_root: Option<&Path>,
    repo_name: &str,
    test_file: &str,
) -> Option<String> {
    let norm_path = test_file.replace('\\', "/");

    // 1. Rust / Cargo
    if norm_path.ends_with(".rs") {
        let is_cargo_repo = if let Some(root) = repo_root {
            root.join("Cargo.toml").is_file()
                || root.join(repo_name).join("Cargo.toml").is_file()
                || root
                    .join(".ivar/repos")
                    .join(repo_name)
                    .join("main/Cargo.toml")
                    .is_file()
                || root
                    .join(".ivar/repos")
                    .join(repo_name)
                    .join("Cargo.toml")
                    .is_file()
        } else {
            true
        };

        if is_cargo_repo {
            // Check integration test file under `tests/`
            if let Some(rest) = norm_path
                .strip_prefix("tests/unit/")
                .and_then(|r| r.strip_suffix(".rs"))
            {
                let mod_path = rest.replace('/', "::");
                return Some(format!("cargo test {mod_path}"));
            }
            if let Some(rest) = norm_path.strip_prefix("tests/") {
                if !rest.contains('/')
                    && let Some(target_name) = rest.strip_suffix(".rs")
                {
                    return Some(format!("cargo test --test {target_name}"));
                }
                if let Some(target_name) = rest.split('/').next() {
                    let target_clean = target_name.trim_end_matches(".rs");
                    if !target_clean.is_empty() {
                        return Some(format!("cargo test --test {target_clean}"));
                    }
                }
            }
            // Unit test inside src/ or others
            if let Some(rest) = norm_path
                .strip_prefix("src/")
                .and_then(|r| r.strip_suffix(".rs"))
            {
                let mod_path = rest.replace('/', "::");
                return Some(format!("cargo test {mod_path}"));
            }
        }
    }

    // 2. Node / TS / JS
    if (norm_path.ends_with(".ts")
        || norm_path.ends_with(".tsx")
        || norm_path.ends_with(".js")
        || norm_path.ends_with(".jsx"))
        && let Some(root) = repo_root
    {
        let pkg_path = if root.join("package.json").is_file() {
            Some(root.join("package.json"))
        } else if root.join(repo_name).join("package.json").is_file() {
            Some(root.join(repo_name).join("package.json"))
        } else if root
            .join(".ivar/repos")
            .join(repo_name)
            .join("main/package.json")
            .is_file()
        {
            Some(
                root.join(".ivar/repos")
                    .join(repo_name)
                    .join("main/package.json"),
            )
        } else if root
            .join(".ivar/repos")
            .join(repo_name)
            .join("package.json")
            .is_file()
        {
            Some(
                root.join(".ivar/repos")
                    .join(repo_name)
                    .join("package.json"),
            )
        } else {
            None
        };

        if let Some(pkg_file) = pkg_path
            && let Ok(content) = std::fs::read_to_string(&pkg_file)
        {
            if content.contains("\"vitest\"") {
                return Some(format!("npx vitest run {norm_path}"));
            } else if content.contains("\"jest\"") {
                return Some(format!("npx jest {norm_path}"));
            } else if content.contains("\"scripts\"") && content.contains("\"test\"") {
                return Some(format!("npm test -- {norm_path}"));
            }
        }
    }

    None
}

/// Finds all test files affected by changes in the given source files.
///
/// Walks reverse dependencies (incoming `IMPORTS`, `CALLS`, and `CROSS_IMPORTS` edges)
/// across symbols and files up to `max_depth` hops, constructing causal explanations
/// and focused test recommendations.
type BfsQueueItem = (i64, String, String, Vec<CausalStep>, HashSet<i64>);

pub fn find_affected_tests(
    db: &GraphDb,
    changed_files: &[String],
    repo: Option<&str>,
    max_depth: usize,
) -> Result<AffectedResult, AffectedError> {
    find_affected_tests_with_root(db, None, changed_files, repo, max_depth)
}

/// Finds all test files affected by changes in the given source files, optionally using hall_root
/// to inspect repository configurations for focused test command derivation.
pub fn find_affected_tests_with_root(
    db: &GraphDb,
    hall_root: Option<&Path>,
    changed_files: &[String],
    repo: Option<&str>,
    max_depth: usize,
) -> Result<AffectedResult, AffectedError> {
    if max_depth == 0 {
        return Err(AffectedError::InvalidDepth(max_depth));
    }

    if changed_files.is_empty() {
        return Ok(AffectedResult {
            changed_files: Vec::new(),
            affected_test_files: Vec::new(),
            recommendations: Vec::new(),
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
            recommendations: Vec::new(),
        });
    }

    // Step 1: Find file IDs for all changed files
    let mut target_files = Vec::new();

    for path in &normalized_changed {
        let mut stmt = conn.prepare_cached(
            "SELECT id, repo, path FROM files WHERE (?1 IS NULL OR repo = ?1) AND (path = ?2 OR (repo || '/' || path) = ?2 OR path LIKE ?3)",
        )?;
        let like_pattern = format!("%/{}", path);
        let mut rows = stmt.query(params![repo, path, like_pattern])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let r: String = row.get(1)?;
            let file_path: String = row.get(2)?;
            target_files.push(TargetFileInfo {
                id,
                repo: r,
                path: file_path,
            });
        }
    }

    let mut recommendations_map: BTreeMap<String, AffectedRecommendation> = BTreeMap::new();
    let mut affected_tests_set = HashSet::new();

    // Direct test changes: if a test file itself changed, it is directly affected
    for tf in &target_files {
        if is_test_file(&tf.path) {
            affected_tests_set.insert(tf.path.clone());
            let cmd = derive_test_command(hall_root, &tf.repo, &tf.path);
            let rec = AffectedRecommendation {
                repo: tf.repo.clone(),
                test_file: tf.path.clone(),
                causal_path: Vec::new(),
                direct_change: true,
                hop_count: 0,
                edge_kind: EdgeKind::Other("changed".to_owned()),
                provenance: Provenance::Extracted,
                confidence: 1.0,
                reason: "direct change to test file".to_owned(),
                command: cmd,
            };
            recommendations_map.insert(tf.path.clone(), rec);
        }
    }

    // Step 2: Reverse traversal to find all affected test files and incoming causal paths
    if !target_files.is_empty() {
        // Collect reverse edges layer by layer up to max_depth
        let edge_query_sql = "
            SELECT
                e.file_id AS from_file_id,
                f_from.repo AS from_repo,
                f_from.path AS from_path,
                s_from.name AS from_symbol_name,
                s_to.file_id AS to_file_id,
                f_to.repo AS to_repo,
                f_to.path AS to_path,
                s_to.name AS to_symbol_name,
                e.kind AS edge_kind,
                e.provenance,
                e.confidence,
                e.line
            FROM edges e
            JOIN files f_from ON e.file_id = f_from.id
            LEFT JOIN symbols s_from ON e.from_symbol_id = s_from.id
            LEFT JOIN symbols s_to ON (
                e.to_symbol_id = s_to.id
                OR (e.to_symbol_id IS NULL AND e.to_name = s_to.name)
            )
            JOIN files f_to ON s_to.file_id = f_to.id
            WHERE s_to.file_id = ?1
              AND (?2 IS NULL OR e.repo = ?2)
        ";

        for tf in &target_files {
            // BFS queue: (current_file_id, current_repo, current_path, accumulated_path, visited_files)
            let mut visited = HashSet::new();
            visited.insert(tf.id);

            let mut queue: std::collections::VecDeque<BfsQueueItem> =
                std::collections::VecDeque::new();
            queue.push_back((tf.id, tf.repo.clone(), tf.path.clone(), Vec::new(), visited));

            while let Some((curr_fid, _curr_repo, curr_path, path_so_far, visited_set)) =
                queue.pop_front()
            {
                if path_so_far.len() >= max_depth {
                    continue;
                }

                let mut stmt = conn.prepare_cached(edge_query_sql)?;
                let mut rows = stmt.query(params![curr_fid, repo])?;

                let mut outgoing_edges = Vec::new();
                while let Some(row) = rows.next()? {
                    let from_fid: i64 = row.get(0)?;
                    let from_repo: String = row.get(1)?;
                    let from_path: String = row.get(2)?;
                    let from_sym: Option<String> = row.get(3)?;
                    let to_fid: i64 = row.get(4)?;
                    let to_repo: String = row.get(5)?;
                    let to_path: String = row.get(6)?;
                    let to_sym: Option<String> = row.get(7)?;
                    let kind_raw: String = row.get(8)?;
                    let prov_raw: String = row.get(9)?;
                    let conf: f64 = row.get(10)?;
                    let line: i64 = row.get(11)?;

                    outgoing_edges.push(RawCausalEdge {
                        from_file_id: from_fid,
                        from_repo,
                        from_path,
                        from_symbol_name: from_sym,
                        _to_file_id: to_fid,
                        _to_repo: to_repo,
                        to_path,
                        to_symbol_name: to_sym,
                        edge_kind: parse_edge_kind(&kind_raw),
                        provenance: parse_provenance(&prov_raw),
                        confidence: conf,
                        line: line as usize,
                    });
                }

                for edge in outgoing_edges {
                    if visited_set.contains(&edge.from_file_id) {
                        continue;
                    }

                    let source_desc = edge
                        .from_symbol_name
                        .as_deref()
                        .map(|s| format!("{}:{}", edge.from_path, s))
                        .unwrap_or_else(|| edge.from_path.clone());
                    let target_desc = edge
                        .to_symbol_name
                        .as_deref()
                        .map(|s| format!("{}:{}", edge.to_path, s))
                        .unwrap_or_else(|| edge.to_path.clone());

                    let step = CausalStep {
                        source: source_desc,
                        target: target_desc,
                        edge_kind: edge.edge_kind.clone(),
                        provenance: edge.provenance,
                        confidence: edge.confidence,
                        line: edge.line,
                    };

                    let mut next_path = path_so_far.clone();
                    next_path.push(step);

                    let hops = next_path.len();

                    if is_test_file(&edge.from_path) {
                        affected_tests_set.insert(edge.from_path.clone());

                        let Some(primary_step) = next_path.first() else {
                            continue;
                        };
                        let edge_kind = primary_step.edge_kind.clone();
                        let provenance = primary_step.provenance;
                        // Aggregate confidence along the path
                        let confidence = next_path.iter().fold(1.0, |acc, s| acc * s.confidence);

                        let reason = if hops == 1 {
                            format!("{} {} (1 hop)", edge_kind.as_str(), tf.path)
                        } else {
                            format!(
                                "transitively depends on {} ({} hops via {})",
                                tf.path, hops, curr_path
                            )
                        };

                        let cmd = derive_test_command(hall_root, &edge.from_repo, &edge.from_path);

                        let candidate = AffectedRecommendation {
                            repo: edge.from_repo.clone(),
                            test_file: edge.from_path.clone(),
                            causal_path: next_path.clone(),
                            direct_change: false,
                            hop_count: hops,
                            edge_kind,
                            provenance,
                            confidence,
                            reason,
                            command: cmd,
                        };

                        // Deterministic best path selection:
                        // 1. Shorter hop_count wins
                        // 2. Higher confidence wins
                        // 3. Lexicographical tie-breaker
                        let should_replace = match recommendations_map.get(&edge.from_path) {
                            None => true,
                            Some(existing) => {
                                if existing.direct_change {
                                    false
                                } else if candidate.hop_count < existing.hop_count {
                                    true
                                } else if candidate.hop_count == existing.hop_count {
                                    if candidate.confidence > existing.confidence {
                                        true
                                    } else if (candidate.confidence - existing.confidence).abs()
                                        < f64::EPSILON
                                    {
                                        candidate.reason < existing.reason
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                        };

                        if should_replace {
                            recommendations_map.insert(edge.from_path.clone(), candidate);
                        }
                    }

                    let mut next_visited = visited_set.clone();
                    next_visited.insert(edge.from_file_id);
                    queue.push_back((
                        edge.from_file_id,
                        edge.from_repo,
                        edge.from_path,
                        next_path,
                        next_visited,
                    ));
                }
            }
        }
    }

    let mut affected_test_files: Vec<String> = affected_tests_set.into_iter().collect();
    affected_test_files.sort();

    let mut recommendations: Vec<AffectedRecommendation> =
        recommendations_map.into_values().collect();
    recommendations.sort_by(|a, b| a.test_file.cmp(&b.test_file));

    Ok(AffectedResult {
        changed_files: normalized_changed,
        affected_test_files,
        recommendations,
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/graph/affected.rs"]
mod tests;
