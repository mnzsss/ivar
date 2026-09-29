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
    pub target_repos: Vec<String>,
    pub path_tokens: Vec<String>,
    pub resolved_paths: Vec<ResolvedPath>,
    pub search_terms: Vec<String>,
    pub route_intent: bool,
}

/// Checks if a token is identifier-shaped (contains an uppercase letter after a lowercase one,
/// contains an underscore, or is a dotted filename basename).
pub(crate) fn is_identifier_shaped(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.contains('_') {
        return true;
    }
    let mut saw_lower = false;
    for c in token.chars() {
        if c.is_lowercase() {
            saw_lower = true;
        } else if saw_lower && c.is_uppercase() {
            return true;
        }
    }
    if token.contains('.')
        && token
            .split('.')
            .next_back()
            .is_some_and(|ext| !ext.is_empty())
    {
        return true;
    }
    false
}

pub fn is_path_like(token: &str) -> bool {
    token.contains('/')
        || token.contains('\\')
        || token.starts_with('.')
        || token.contains('-')
        || (token.contains('.')
            && token.split('.').next_back().is_some_and(|ext| {
                matches!(
                    ext,
                    "rs" | "ts"
                        | "tsx"
                        | "js"
                        | "jsx"
                        | "mjs"
                        | "cjs"
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
                        | "txt"
                        | "lock"
                        | "dockerfile"
                        | "xml"
                        | "html"
                        | "css"
                        | "scss"
                        | "json"
                        | "toml"
                        | "yaml"
                        | "yml"
                        | "md"
                )
            }))
}

/// Splits an optional trailing line range suffix `:<start>-<end>` (1-based inclusive)
/// from a file path token. Suffix must end in `:\d+-\d+`.
/// Returns `(path, Some((start, end)))` or `(token, None)`.
pub fn split_line_range(token: &str) -> (&str, Option<(usize, usize)>) {
    if let Some(colon_pos) = token.rfind(':') {
        // Guard against Windows drive paths (e.g. C:\foo) where colon is at index 1 and followed by slash
        if colon_pos == 1
            && token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            return (token, None);
        }
        let (path_part, suffix) = token.split_at(colon_pos);
        let range_part = &suffix[1..];
        if let Some((start_str, end_str)) = range_part.split_once('-')
            && !start_str.is_empty()
            && !end_str.is_empty()
            && start_str.chars().all(|c| c.is_ascii_digit())
            && end_str.chars().all(|c| c.is_ascii_digit())
            && let (Ok(start), Ok(end)) = (start_str.parse::<usize>(), end_str.parse::<usize>())
        {
            return (path_part, Some((start, end)));
        }
    }
    (token, None)
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

    let mut target_repos: Vec<String> = repo.map(|r| vec![r.to_owned()]).unwrap_or_default();
    let mut unconsumed_tokens = Vec::new();

    // 1. Separate recognized repo names
    for token in tokens {
        let clean_token = token.trim_matches(|c: char| {
            c == ','
                || c == ';'
                || c == ':'
                || c == '"'
                || c == '\''
                || c == '`'
                || c == '('
                || c == ')'
                || c == '{'
                || c == '}'
        });
        if clean_token.is_empty() {
            continue;
        }
        if !clean_token.contains('/')
            && !clean_token.contains('\\')
            && let Ok(Some(repo_row)) = db.get_visible_repo(clean_token)
        {
            if !target_repos.contains(&repo_row.id) {
                target_repos.push(repo_row.id);
            }
            continue;
        }
        unconsumed_tokens.push(clean_token);
    }

    let mut path_tokens = Vec::new();
    let mut resolved_paths = Vec::new();
    let mut remaining_words = Vec::new();
    let mut route_intent = false;

    for clean_token in unconsumed_tokens {
        if is_route_intent_term(clean_token) {
            route_intent = true;
        }

        let (path_token, _line_range) = split_line_range(clean_token);
        let is_pl = is_path_like(path_token);
        let trimmed = path_token.trim_start_matches("./");

        if is_pl {
            path_tokens.push(path_token.to_owned());
            if target_repos.is_empty() {
                if let Some(resolved) = resolve_path_token(db, conn, None, trimmed)? {
                    resolved_paths.push(resolved);
                }
            } else {
                for target_repo in &target_repos {
                    if let Some(resolved) =
                        resolve_path_token(db, conn, Some(target_repo.as_str()), trimmed)?
                    {
                        resolved_paths.push(resolved);
                    }
                }
            }
        }
        if !clean_token.contains('/') {
            let clean_term = clean_token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if !clean_term.is_empty() {
                remaining_words.push(clean_term.to_owned());
            }
            if clean_token != clean_term && !clean_token.is_empty() {
                remaining_words.push(clean_token.to_owned());
            }
        }
    }
    Ok(ParsedExploreQuery {
        raw_query: query.to_owned(),
        target_repos,
        path_tokens,
        resolved_paths,
        search_terms: remaining_words,
        route_intent,
    })
}

fn resolve_path_token(
    db: &GraphDb,
    conn: &rusqlite::Connection,
    repo: Option<&str>,
    trimmed: &str,
) -> Result<Option<ResolvedPath>, QueryError> {
    // 1. Check exact match: f.path = ? OR repo = repo AND f.path = subpath
    let mut exact_stmt = conn.prepare_cached(
        "SELECT id, repo, path FROM visible_files WHERE (?1 IS NULL OR repo = ?1) AND path = ?2 LIMIT 2",
    )?;
    let exact_matches: Vec<(i64, String, String)> = exact_stmt
        .query_map(params![repo, trimmed], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    if let [(file_id, r, p)] = exact_matches.as_slice() {
        return Ok(Some(ResolvedPath::ExactFile {
            file_id: *file_id,
            repo: r.clone(),
            path: p.clone(),
        }));
    }

    if let Some((repo_cand, rest)) = trimmed.split_once('/')
        && (repo.is_none() || repo == Some(repo_cand))
    {
        let mut repo_prefix_stmt = conn.prepare_cached(
            "SELECT id, repo, path FROM visible_files WHERE repo = ?1 AND path = ?2 LIMIT 2",
        )?;
        let repo_prefix_matches: Vec<(i64, String, String)> = repo_prefix_stmt
            .query_map(params![repo_cand, rest], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<_, _>>()?;

        if let [(file_id, r, p)] = repo_prefix_matches.as_slice() {
            return Ok(Some(ResolvedPath::ExactFile {
                file_id: *file_id,
                repo: r.clone(),
                path: p.clone(),
            }));
        }
    }

    // 2. Check workspace-relative / suffix match
    let mut rel_stmt = conn.prepare_cached(
        "SELECT id, repo, path FROM visible_files
         WHERE (?1 IS NULL OR repo = ?1)
           AND (
             path = ?2
             OR path LIKE '%/' || ?2
             OR ?2 LIKE '%/' || path
           )
         LIMIT 10",
    )?;
    let rel_matches: Vec<(i64, String, String)> = rel_stmt
        .query_map(params![repo, trimmed], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    if let [(file_id, r, p)] = rel_matches.as_slice() {
        return Ok(Some(ResolvedPath::WorkspaceRelative {
            file_id: *file_id,
            repo: r.clone(),
            path: p.clone(),
        }));
    } else if rel_matches.len() > 1 && !trimmed.contains('/') && !trimmed.contains('\\') {
        return Ok(Some(ResolvedPath::AmbiguousBasename { files: rel_matches }));
    }

    resolve_directory_or_basename(db, conn, repo, trimmed)
}

fn resolve_directory_or_basename(
    db: &GraphDb,
    conn: &rusqlite::Connection,
    repo: Option<&str>,
    trimmed: &str,
) -> Result<Option<ResolvedPath>, QueryError> {
    // 3. Basename or Directory/Subtree match
    let clean_dir = trimmed.trim_end_matches('/');
    let mut dir_stmt = conn.prepare_cached(
        "SELECT id, repo, path FROM visible_files
         WHERE (?1 IS NULL OR repo = ?1)
           AND (
             path LIKE ?2 || '/%'
             OR path LIKE '%/' || ?2 || '/%'
           )
         LIMIT 50",
    )?;
    let mut dir_matches: Vec<(i64, String, String)> = dir_stmt
        .query_map(params![repo, clean_dir], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    if dir_matches.is_empty() {
        let mut subtree_stmt = conn.prepare_cached(
            "SELECT id, repo, path FROM visible_files
             WHERE (?1 IS NULL OR repo = ?1) AND path LIKE ?2 || '/%'
             LIMIT 50",
        )?;
        for (slash, _) in clean_dir.match_indices('/') {
            let named_repo = match clean_dir[..slash].rsplit('/').next() {
                Some(segment) if repo.is_none() && db.get_visible_repo(segment)?.is_some() => {
                    Some(segment)
                }
                _ => None,
            };
            dir_matches = subtree_stmt
                .query_map(params![repo.or(named_repo), &clean_dir[slash + 1..]], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<Result<_, _>>()?;
            if !dir_matches.is_empty() {
                break;
            }
        }
    }

    if !dir_matches.is_empty() {
        return Ok(Some(ResolvedPath::DirectorySubtree { files: dir_matches }));
    }

    // 4. Basename lookup across database
    let mut base_stmt = conn.prepare_cached(
        "SELECT id, repo, path FROM visible_files
         WHERE (?1 IS NULL OR repo = ?1)
           AND (path = ?2 OR path LIKE '%/' || ?2)
         LIMIT 10",
    )?;
    let base_matches: Vec<(i64, String, String)> = base_stmt
        .query_map(params![repo, trimmed], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    if let [(file_id, r, p)] = base_matches.as_slice() {
        Ok(Some(ResolvedPath::UnambiguousBasename {
            file_id: *file_id,
            repo: r.clone(),
            path: p.clone(),
        }))
    } else if base_matches.len() > 1 {
        Ok(Some(ResolvedPath::AmbiguousBasename {
            files: base_matches,
        }))
    } else {
        Ok(None)
    }
}
