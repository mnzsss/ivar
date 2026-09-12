use rusqlite::params;

use crate::action::graph::query::types::QueryError;
use crate::store::graph::db::GraphDb;

/// Resolved path target from query parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    /// Exact file match (pinned).
    ExactFile {
        file_id: i64,
        repo: String,
        path: String,
    },
    /// Workspace-relative file match (pinned).
    WorkspaceRelative {
        file_id: i64,
        repo: String,
        path: String,
    },
    /// Single unambiguous basename match across the database (pinned).
    UnambiguousBasename {
        file_id: i64,
        repo: String,
        path: String,
    },
    /// Multiple files matched a basename (unpinned candidates).
    AmbiguousBasename { files: Vec<(i64, String, String)> },
    /// Directory or subtree match (unpinned candidate files).
    DirectorySubtree { files: Vec<(i64, String, String)> },
}

impl ResolvedPath {
    pub fn is_pinned(&self) -> bool {
        matches!(
            self,
            Self::ExactFile { .. }
                | Self::WorkspaceRelative { .. }
                | Self::UnambiguousBasename { .. }
        )
    }

    pub fn files(&self) -> Vec<(i64, String, String)> {
        match self {
            Self::ExactFile {
                file_id,
                repo,
                path,
            }
            | Self::WorkspaceRelative {
                file_id,
                repo,
                path,
            }
            | Self::UnambiguousBasename {
                file_id,
                repo,
                path,
            } => {
                vec![(*file_id, repo.clone(), path.clone())]
            }
            Self::AmbiguousBasename { files } | Self::DirectorySubtree { files } => files.clone(),
        }
    }
}

/// Parsed components of an explore query.
#[derive(Debug, Clone, Default)]
pub struct ParsedExploreQuery {
    pub raw_query: String,
    pub path_tokens: Vec<String>,
    pub resolved_paths: Vec<ResolvedPath>,
    pub search_terms: Vec<String>,
    pub route_intent: bool,
}

/// Checks whether a token looks like a file path or directory.
pub fn is_path_like(token: &str) -> bool {
    let t = token.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains('/') || t.contains('\\') {
        return true;
    }
    matches!(
        t.rsplit_once('.').map(|(_, ext)| ext),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "json"
                | "md"
                | "toml"
                | "yaml"
                | "yml"
                | "py"
                | "go"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "java"
                | "kt"
                | "swift"
                | "proto"
                | "sql"
                | "graphql"
                | "sh"
        )
    )
}

/// Detects if terms express route/API intent.
fn is_route_intent_term(term: &str) -> bool {
    let lower = term.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "route" | "routes" | "endpoint" | "endpoints" | "api"
    )
}

/// Resolves path tokens against the database to identify exact files, workspace-relative
/// paths, unambiguous basenames, ambiguous basenames, or directory subtrees.
pub fn resolve_query_paths(
    db: &GraphDb,
    query: &str,
    repo: Option<&str>,
) -> Result<ParsedExploreQuery, QueryError> {
    let conn = db.conn();
    let tokens: Vec<&str> = query.split_whitespace().collect();

    let mut path_tokens = Vec::new();
    let mut resolved_paths = Vec::new();
    let mut remaining_words = Vec::new();
    let mut route_intent = false;

    for token in tokens {
        let clean_token =
            token.trim_matches(|c: char| c == ',' || c == ';' || c == ':' || c == '"' || c == '\'');
        if clean_token.is_empty() {
            continue;
        }

        if is_route_intent_term(clean_token) {
            route_intent = true;
        }

        if is_path_like(clean_token) {
            path_tokens.push(clean_token.to_owned());
            let trimmed = clean_token.trim_start_matches("./");

            // 1. Check exact match: f.path = ?
            let mut exact_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM visible_files WHERE (?1 IS NULL OR repo = ?1) AND path = ?2 LIMIT 2",
            )?;
            let exact_matches: Vec<(i64, String, String)> = exact_stmt
                .query_map(params![repo, trimmed], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let [(file_id, r, p)] = exact_matches.as_slice() {
                resolved_paths.push(ResolvedPath::ExactFile {
                    file_id: *file_id,
                    repo: r.clone(),
                    path: p.clone(),
                });
                continue;
            }

            // 2. Check workspace-relative / suffix match
            let mut rel_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM visible_files
                 WHERE (?1 IS NULL OR repo = ?1)
                   AND (
                     path = ?2
                     OR path LIKE '%/' || ?2 ESCAPE '\\'
                     OR ?2 LIKE '%/' || path ESCAPE '\\'
                   )
                 LIMIT 10",
            )?;
            let rel_matches: Vec<(i64, String, String)> = rel_stmt
                .query_map(params![repo, trimmed], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let [(file_id, r, p)] = rel_matches.as_slice() {
                resolved_paths.push(ResolvedPath::WorkspaceRelative {
                    file_id: *file_id,
                    repo: r.clone(),
                    path: p.clone(),
                });
                continue;
            } else if rel_matches.len() > 1 && !trimmed.contains('/') && !trimmed.contains('\\') {
                resolved_paths.push(ResolvedPath::AmbiguousBasename { files: rel_matches });
                continue;
            }

            // 3. Basename or Directory/Subtree match
            let clean_dir = trimmed.trim_end_matches('/');
            let mut dir_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM visible_files
                 WHERE (?1 IS NULL OR repo = ?1)
                   AND (
                     path LIKE ?2 || '/%' ESCAPE '\\'
                     OR path LIKE '%/' || ?2 || '/%' ESCAPE '\\'
                   )
                 LIMIT 50",
            )?;
            let mut dir_matches: Vec<(i64, String, String)> = dir_stmt
                .query_map(params![repo, clean_dir], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if dir_matches.is_empty() {
                let mut subtree_stmt = conn.prepare_cached(
                    "SELECT id, repo, path FROM visible_files
                     WHERE (?1 IS NULL OR repo = ?1) AND path LIKE ?2 || '/%' ESCAPE '\\'
                     LIMIT 50",
                )?;
                for (slash, _) in clean_dir.match_indices('/') {
                    let named_repo = match clean_dir[..slash].rsplit('/').next() {
                        Some(segment) if repo.is_none() && db.get_repo(segment)?.is_some() => {
                            Some(segment)
                        }
                        _ => None,
                    };
                    dir_matches = subtree_stmt
                        .query_map(params![repo.or(named_repo), &clean_dir[slash + 1..]], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                        })?
                        .filter_map(|r| r.ok())
                        .collect();
                    if !dir_matches.is_empty() {
                        break;
                    }
                }
            }

            if !dir_matches.is_empty() {
                resolved_paths.push(ResolvedPath::DirectorySubtree { files: dir_matches });
                continue;
            }

            // 4. Basename lookup across database
            let mut base_stmt = conn.prepare_cached(
                "SELECT id, repo, path FROM visible_files
                 WHERE (?1 IS NULL OR repo = ?1)
                   AND (path = ?2 OR path LIKE '%/' || ?2 ESCAPE '\\')
                 LIMIT 10",
            )?;
            let base_matches: Vec<(i64, String, String)> = base_stmt
                .query_map(params![repo, trimmed], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            if let [(file_id, r, p)] = base_matches.as_slice() {
                resolved_paths.push(ResolvedPath::UnambiguousBasename {
                    file_id: *file_id,
                    repo: r.clone(),
                    path: p.clone(),
                });
            } else if base_matches.len() > 1 {
                resolved_paths.push(ResolvedPath::AmbiguousBasename {
                    files: base_matches,
                });
            }
        } else {
            let clean_term = clean_token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if !clean_term.is_empty() {
                remaining_words.push(clean_term.to_owned());
            }
        }
    }

    Ok(ParsedExploreQuery {
        raw_query: query.to_owned(),
        path_tokens,
        resolved_paths,
        search_terms: remaining_words,
        route_intent,
    })
}
