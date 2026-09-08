//! Edge table storage and linking operations.

use rusqlite::params;

use super::GraphDb;
use super::types::{Result, edge_kind_to_str, provenance_to_str};
use crate::domain::graph::Edge;

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
}
