//! Edge table storage and linking operations.

use rusqlite::params;

use super::GraphDb;
use super::types::{Result, edge_kind_to_str, provenance_to_str};
use crate::domain::graph::{Edge, Symbol};

/// Result of finding hierarchy for a symbol: symbol, file path, base types, and subtype/implementor names.
pub type HierarchyRecord = (Symbol, String, Vec<String>, Vec<String>);

impl GraphDb {
    /// Bulk inserts edges in a single transaction and returns their generated IDs.
    pub fn insert_edges(&self, edges: &[Edge]) -> Result<Vec<i64>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let res = (|| -> Result<Vec<i64>> {
            let mut stmt = self.conn.prepare_cached(
                "INSERT INTO edges (repo, file_id, from_symbol_id, to_symbol_id, to_name, kind, provenance, line, col, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 RETURNING id",
            )?;
            let mut ids = Vec::with_capacity(edges.len());
            for edge in edges {
                let kind_str = edge_kind_to_str(&edge.kind);
                let prov_str = provenance_to_str(&edge.provenance);
                let id: i64 = stmt.query_row(
                    params![
                        &edge.repo,
                        edge.file_id,
                        edge.from_symbol_id,
                        edge.to_symbol_id,
                        &edge.to_name,
                        kind_str.as_ref(),
                        prov_str,
                        edge.line as i64,
                        edge.col as i64,
                        edge.confidence,
                    ],
                    |row| row.get(0),
                )?;
                ids.push(id);
            }
            Ok(ids)
        })();

        match res {
            Ok(ids) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(ids)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Deletes all edges originating from or associated with a specific file ID.
    pub fn delete_edges_for_file(&self, file_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM edges WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    /// Re-links dangling edges where `to_symbol_id` is null by matching `to_name` with symbols in the same repository.
    pub fn relink_dangling_edges(&self, repo: &str) -> Result<usize> {
        let count = self.conn.execute(
            "UPDATE edges
             SET to_symbol_id = (
                 SELECT s.id FROM symbols s
                 WHERE s.repo = edges.repo
                   AND s.name = edges.to_name
                 ORDER BY s.is_exported DESC, s.id ASC
                 LIMIT 1
             )
             WHERE repo = ?1
               AND to_symbol_id IS NULL
               AND to_name IS NOT NULL
               AND EXISTS (
                 SELECT 1 FROM symbols s
                 WHERE s.repo = edges.repo
                   AND s.name = edges.to_name
               )",
            params![repo],
        )?;
        Ok(count)
    }

    /// Finds base types and implementations/subtypes for a symbol.
    pub fn find_hierarchy(
        &self,
        symbol_name: &str,
        repo: Option<&str>,
    ) -> Result<Option<HierarchyRecord>> {
        use super::types::parse_symbol_kind;
        use crate::domain::graph::{Span, Symbol};

        let sym_row = match repo {
            Some(r) => {
                let mut stmt = self.conn.prepare(
                    "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                            s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
                            f.path
                     FROM symbols s
                     JOIN files f ON s.file_id = f.id
                     WHERE s.name = ?1 AND s.repo = ?2
                     ORDER BY s.is_exported DESC, s.id ASC
                     LIMIT 1",
                )?;
                stmt.query_row(params![symbol_name, r], |row| {
                    let id: i64 = row.get(0)?;
                    let file_id: i64 = row.get(1)?;
                    let repo: String = row.get(2)?;
                    let name: String = row.get(3)?;
                    let kind_raw: String = row.get(4)?;
                    let scope: Option<String> = row.get(5)?;
                    let signature: Option<String> = row.get(6)?;
                    let docstring: Option<String> = row.get(7)?;
                    let start_line: i64 = row.get(8)?;
                    let start_col: i64 = row.get(9)?;
                    let end_line: i64 = row.get(10)?;
                    let end_col: i64 = row.get(11)?;
                    let is_exported: i64 = row.get(12)?;
                    let complexity: Option<i64> = row.get(13)?;
                    let file_path: String = row.get(14)?;

                    Ok((
                        Symbol {
                            id: Some(id),
                            file_id: Some(file_id),
                            repo,
                            name,
                            kind: parse_symbol_kind(&kind_raw),
                            scope,
                            signature,
                            docstring,
                            span: Span::new(
                                start_line as usize,
                                start_col as usize,
                                end_line as usize,
                                end_col as usize,
                            ),
                            is_exported: is_exported != 0,
                            complexity: complexity.map(|c| c as u32),
                        },
                        file_path,
                    ))
                })
                .ok()
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring,
                            s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity,
                            f.path
                     FROM symbols s
                     JOIN files f ON s.file_id = f.id
                     WHERE s.name = ?1
                     ORDER BY s.is_exported DESC, s.id ASC
                     LIMIT 1",
                )?;
                stmt.query_row(params![symbol_name], |row| {
                    let id: i64 = row.get(0)?;
                    let file_id: i64 = row.get(1)?;
                    let repo: String = row.get(2)?;
                    let name: String = row.get(3)?;
                    let kind_raw: String = row.get(4)?;
                    let scope: Option<String> = row.get(5)?;
                    let signature: Option<String> = row.get(6)?;
                    let docstring: Option<String> = row.get(7)?;
                    let start_line: i64 = row.get(8)?;
                    let start_col: i64 = row.get(9)?;
                    let end_line: i64 = row.get(10)?;
                    let end_col: i64 = row.get(11)?;
                    let is_exported: i64 = row.get(12)?;
                    let complexity: Option<i64> = row.get(13)?;
                    let file_path: String = row.get(14)?;

                    Ok((
                        Symbol {
                            id: Some(id),
                            file_id: Some(file_id),
                            repo,
                            name,
                            kind: parse_symbol_kind(&kind_raw),
                            scope,
                            signature,
                            docstring,
                            span: Span::new(
                                start_line as usize,
                                start_col as usize,
                                end_line as usize,
                                end_col as usize,
                            ),
                            is_exported: is_exported != 0,
                            complexity: complexity.map(|c| c as u32),
                        },
                        file_path,
                    ))
                })
                .ok()
            }
        };

        let (symbol, file_path) = match sym_row {
            Some(item) => item,
            None => return Ok(None),
        };

        let sym_id = symbol.id.unwrap_or_default();

        // Find bases (what this symbol implements or inherits from)
        let mut stmt_bases = self.conn.prepare(
            "SELECT DISTINCT COALESCE(s2.name, e.to_name)
             FROM edges e
             LEFT JOIN symbols s2 ON e.to_symbol_id = s2.id
             WHERE e.from_symbol_id = ?1
               AND e.kind IN ('IMPLEMENTS', 'INHERITS')
               AND COALESCE(s2.name, e.to_name) IS NOT NULL",
        )?;
        let base_rows = stmt_bases.query_map(params![sym_id], |row| row.get::<_, String>(0))?;
        let mut bases = Vec::new();
        for b in base_rows {
            bases.push(b?);
        }

        // Find implementations/subtypes (what implements or inherits from this symbol)
        let mut stmt_derived = self.conn.prepare(
            "SELECT DISTINCT s2.name
             FROM edges e
             JOIN symbols s2 ON e.from_symbol_id = s2.id
             WHERE (e.to_symbol_id = ?1 OR e.to_name = ?2)
               AND e.kind IN ('IMPLEMENTS', 'INHERITS')",
        )?;
        let derived_rows =
            stmt_derived.query_map(params![sym_id, &symbol.name], |row| row.get::<_, String>(0))?;
        let mut implementations = Vec::new();
        for d in derived_rows {
            implementations.push(d?);
        }

        Ok(Some((symbol, file_path, bases, implementations)))
    }
}
