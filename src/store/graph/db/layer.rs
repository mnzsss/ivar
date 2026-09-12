//! Layer records, tombstones CRUD, session_layers temp table, and visible_* views.

use super::GraphDb;
use super::types::{Result, now_timestamp};
use crate::domain::graph::LayerStats;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRecord {
    pub id: i64,
    pub feature: String,
    pub repo: String,
    pub worktree: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
    pub fingerprint: Option<String>,
    pub indexed_at: i64,
}

pub fn setup_views(conn: &Connection) -> Result<()> {
    conn.execute_batch(r#"
        CREATE TEMP TABLE IF NOT EXISTS session_layers (
            repo TEXT PRIMARY KEY,
            layer_repo TEXT NOT NULL
        );

        DROP VIEW IF EXISTS visible_edges;
        DROP VIEW IF EXISTS visible_symbols;
        DROP VIEW IF EXISTS visible_files;
        DROP VIEW IF EXISTS visible_repos;

        CREATE TEMP VIEW visible_repos AS
        SELECT r.id, r.root_path, r.default_branch, r.last_indexed_commit
        FROM repos r
        WHERE r.id NOT LIKE '%/%'
          AND r.id NOT IN (SELECT repo FROM session_layers)
        UNION ALL
        SELECT sl.repo AS id, r.root_path, r.default_branch, r.last_indexed_commit
        FROM session_layers sl
        JOIN repos r ON r.id = sl.layer_repo;

        CREATE TEMP VIEW visible_files AS
        SELECT f.id, f.repo, f.path, f.content_hash, f.mtime_ns, f.size_bytes
        FROM files f
        WHERE f.repo NOT LIKE '%/%'
          AND NOT EXISTS (
              SELECT 1 FROM session_layers sl
              LEFT JOIN layer_tombstones lt ON lt.path = f.path AND lt.layer_id IN (
                  SELECT l.id FROM layers l WHERE l.repo = sl.repo AND sl.layer_repo = sl.repo || '/' || l.id
              )
              LEFT JOIN files lf ON lf.repo = sl.layer_repo AND lf.path = f.path
              WHERE sl.repo = f.repo AND (lt.path IS NOT NULL OR lf.id IS NOT NULL)
          )
        UNION ALL
        SELECT f.id, sl.repo AS repo, f.path, f.content_hash, f.mtime_ns, f.size_bytes
        FROM session_layers sl
        JOIN files f ON f.repo = sl.layer_repo;

        CREATE TEMP VIEW visible_symbols AS
        SELECT s.id, s.file_id, s.repo, s.name, s.kind, s.scope, s.signature, s.docstring, s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, s.name_words
        FROM symbols s
        JOIN visible_files vf ON s.file_id = vf.id
        WHERE vf.repo NOT LIKE '%/%' AND s.repo NOT LIKE '%/%'
        UNION ALL
        SELECT s.id, s.file_id, sl.repo AS repo, s.name, s.kind, s.scope, s.signature, s.docstring, s.start_line, s.start_col, s.end_line, s.end_col, s.is_exported, s.complexity, s.name_words
        FROM session_layers sl
        JOIN files f ON f.repo = sl.layer_repo
        JOIN symbols s ON s.file_id = f.id;

        CREATE TEMP VIEW visible_edges AS
        SELECT e.id, e.repo, e.file_id, e.from_symbol_id,
               CASE WHEN to_s.id IS NULL AND e.to_symbol_id IS NOT NULL THEN NULL ELSE e.to_symbol_id END AS to_symbol_id,
               e.to_name, e.kind, e.provenance, e.line, e.col, e.confidence
        FROM edges e
        JOIN visible_files vf ON e.file_id = vf.id
        LEFT JOIN visible_symbols to_s ON e.to_symbol_id = to_s.id
        WHERE e.repo NOT LIKE '%/%'
        UNION ALL
        SELECT e.id, sl.repo AS repo, e.file_id, e.from_symbol_id,
               CASE WHEN to_s.id IS NULL AND e.to_symbol_id IS NOT NULL THEN NULL ELSE e.to_symbol_id END AS to_symbol_id,
               e.to_name, e.kind, e.provenance, e.line, e.col, e.confidence
        FROM session_layers sl
        JOIN files f ON f.repo = sl.layer_repo
        JOIN edges e ON e.file_id = f.id
        LEFT JOIN visible_symbols to_s ON e.to_symbol_id = to_s.id;
    "#)?;
    Ok(())
}

impl GraphDb {
    pub fn ensure_views_base_mode(&self) -> Result<()> {
        setup_views(&self.conn)?;
        self.conn.execute_batch("DELETE FROM session_layers;")?;
        Ok(())
    }

    pub fn configure_session_mode(&self, layers: &[(&str, &str)]) -> Result<()> {
        setup_views(&self.conn)?;
        self.conn.execute_batch("DELETE FROM session_layers;")?;
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO session_layers (repo, layer_repo) VALUES (?1, ?2) ON CONFLICT(repo) DO UPDATE SET layer_repo = excluded.layer_repo"
        )?;
        for (repo, layer_repo) in layers {
            stmt.execute(params![repo, layer_repo])?;
        }
        Ok(())
    }

    pub fn clear_session_layers(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM session_layers;")?;
        Ok(())
    }

    pub fn ensure_layer_record(
        &self,
        feature: &str,
        repo: &str,
        worktree: &str,
        base_commit: &str,
    ) -> Result<i64> {
        let now = now_timestamp();
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO layers (feature, repo, worktree, base_commit, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(feature, repo) DO UPDATE SET
                worktree = excluded.worktree,
                base_commit = excluded.base_commit,
                indexed_at = excluded.indexed_at
             RETURNING id",
        )?;
        let id: i64 = stmt
            .query_row(params![feature, repo, worktree, base_commit, now], |row| {
                row.get(0)
            })?;
        Ok(id)
    }

    pub fn get_layer_record(&self, feature: &str, repo: &str) -> Result<Option<LayerRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, feature, repo, worktree, base_commit, head_commit, fingerprint, indexed_at
             FROM layers WHERE feature = ?1 AND repo = ?2",
        )?;
        let record = stmt
            .query_row(params![feature, repo], |row| {
                Ok(LayerRecord {
                    id: row.get(0)?,
                    feature: row.get(1)?,
                    repo: row.get(2)?,
                    worktree: row.get(3)?,
                    base_commit: row.get(4)?,
                    head_commit: row.get(5)?,
                    fingerprint: row.get(6)?,
                    indexed_at: row.get(7)?,
                })
            })
            .optional()?;
        Ok(record)
    }

    pub fn get_layer_tombstones(&self, layer_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT path FROM layer_tombstones WHERE layer_id = ?1 ORDER BY path",
        )?;
        let paths = stmt
            .query_map(params![layer_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn set_layer_tombstones(&self, layer_id: i64, paths: &[&str]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM layer_tombstones WHERE layer_id = ?1",
            params![layer_id],
        )?;
        let mut stmt = self
            .conn
            .prepare_cached("INSERT INTO layer_tombstones (layer_id, path) VALUES (?1, ?2)")?;
        for p in paths {
            stmt.execute(params![layer_id, p])?;
        }
        Ok(())
    }

    pub fn delete_layer_record(&self, layer_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM layers WHERE id = ?1", params![layer_id])?;
        Ok(())
    }

    pub fn update_layer_fingerprint_and_head(
        &self,
        layer_id: i64,
        fingerprint: &str,
        head_commit: Option<&str>,
    ) -> Result<()> {
        let now = now_timestamp();
        self.conn.execute(
            "UPDATE layers SET fingerprint = ?1, head_commit = ?2, indexed_at = ?3 WHERE id = ?4",
            params![fingerprint, head_commit, now, layer_id],
        )?;
        Ok(())
    }

    pub fn rename_feature_layers(&self, old_feature: &str, new_feature: &str) -> Result<usize> {
        let count = self.conn.execute(
            "UPDATE layers SET feature = ?1 WHERE feature = ?2",
            params![new_feature, old_feature],
        )?;
        Ok(count)
    }

    pub fn drop_feature_layers(&self, feature: &str) -> Result<usize> {
        let layer_ids: Vec<(i64, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, repo FROM layers WHERE feature = ?1")?;
            let rows = stmt.query_map([feature], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut items = Vec::new();
            for item in rows {
                items.push(item?);
            }
            items
        };
        for (id, repo) in &layer_ids {
            let layer_repo = format!("{repo}/{id}");
            let _ = self.delete_repo(&layer_repo);
            let _ = self
                .conn
                .execute("DELETE FROM layer_tombstones WHERE layer_id = ?1", [id]);
            let _ = self.conn.execute("DELETE FROM layers WHERE id = ?1", [id]);
        }
        Ok(layer_ids.len())
    }

    pub fn gc_stale_layers(&self, active_features: &[&str]) -> Result<usize> {
        let all_features: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT DISTINCT feature FROM layers")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            let mut feats = Vec::new();
            for item in rows {
                feats.push(item?);
            }
            feats
        };
        let mut dropped = 0;
        for feat in all_features {
            if !active_features.contains(&feat.as_str()) {
                dropped += self.drop_feature_layers(&feat)?;
            }
        }
        Ok(dropped)
    }

    pub fn get_all_layer_stats(&self) -> Result<Vec<LayerStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.feature, l.repo, l.base_commit, COUNT(f.id) AS file_count
             FROM layers l
             LEFT JOIN files f ON f.repo = l.repo || '/' || l.id
             GROUP BY l.id
             ORDER BY l.feature, l.repo",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(LayerStats {
                feature: r.get(0)?,
                repo: r.get(1)?,
                base_commit: r.get(2)?,
                file_count: r.get::<_, i64>(3)? as usize,
            })
        })?;
        let mut result = Vec::new();
        for item in rows {
            result.push(item?);
        }
        Ok(result)
    }
}
