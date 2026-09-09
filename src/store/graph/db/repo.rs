//! Repository and file metadata storage operations.

use rusqlite::{OptionalExtension, params};

use super::GraphDb;
use super::types::{CleanAllStats, FileRow, RepoCleanStats, RepoRow, Result, now_timestamp};

impl GraphDb {
    /// Inserts or updates repository metadata.
    pub fn insert_repo(
        &self,
        id: &str,
        root_path: &str,
        default_branch: &str,
        commit: Option<&str>,
    ) -> Result<()> {
        let now = now_timestamp();
        self.conn.execute(
            "INSERT INTO repos (id, root_path, default_branch, last_indexed_commit, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                root_path = excluded.root_path,
                default_branch = excluded.default_branch,
                last_indexed_commit = COALESCE(excluded.last_indexed_commit, repos.last_indexed_commit),
                indexed_at = excluded.indexed_at",
            params![id, root_path, default_branch, commit, now],
        )?;
        Ok(())
    }

    /// Fetches repository metadata by repository ID.
    pub fn get_repo(&self, id: &str) -> Result<Option<RepoRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, root_path, default_branch, last_indexed_commit, indexed_at FROM repos WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(params![id], |row| {
                Ok(RepoRow {
                    id: row.get(0)?,
                    root_path: row.get(1)?,
                    default_branch: row.get(2)?,
                    last_indexed_commit: row.get(3)?,
                    indexed_at: row.get(4)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// Updates the last indexed commit and timestamp for a repository.
    pub fn update_repo_commit(&self, id: &str, commit: &str) -> Result<()> {
        let now = now_timestamp();
        self.conn.execute(
            "UPDATE repos SET last_indexed_commit = ?1, indexed_at = ?2 WHERE id = ?3",
            params![commit, now, id],
        )?;
        Ok(())
    }

    /// Fetches the last indexed commit for a repository, if any.
    pub fn get_repo_last_commit(&self, repo_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT last_indexed_commit FROM repos WHERE id = ?1")?;
        let result = stmt
            .query_row(params![repo_id], |row| row.get::<_, Option<String>>(0))
            .optional()?
            .flatten();
        Ok(result)
    }

    /// Inserts or updates file metadata and returns the row ID.
    pub fn upsert_file(
        &self,
        repo: &str,
        path: &str,
        hash: &str,
        mtime_ns: i64,
        size_bytes: i64,
    ) -> Result<i64> {
        let now = now_timestamp();
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO files (repo, path, content_hash, mtime_ns, size_bytes, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(repo, path) DO UPDATE SET
                content_hash = excluded.content_hash,
                mtime_ns = excluded.mtime_ns,
                size_bytes = excluded.size_bytes,
                indexed_at = excluded.indexed_at
             RETURNING id",
        )?;
        let id: i64 = stmt.query_row(
            params![repo, path, hash, mtime_ns, size_bytes, now],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Fetches file metadata by repo and path.
    pub fn get_file(&self, repo: &str, path: &str) -> Result<Option<FileRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at FROM files WHERE repo = ?1 AND path = ?2",
        )?;
        let result = stmt
            .query_row(params![repo, path], |row| {
                Ok(FileRow {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    path: row.get(2)?,
                    content_hash: row.get(3)?,
                    mtime_ns: row.get(4)?,
                    size_bytes: row.get(5)?,
                    indexed_at: row.get(6)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// Fetches all indexed files for a repository.
    pub fn get_files_for_repo(&self, repo: &str) -> Result<Vec<FileRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at FROM files WHERE repo = ?1 ORDER BY path ASC",
        )?;
        let rows = stmt.query_map(params![repo], |row| {
            Ok(FileRow {
                id: row.get(0)?,
                repo: row.get(1)?,
                path: row.get(2)?,
                content_hash: row.get(3)?,
                mtime_ns: row.get(4)?,
                size_bytes: row.get(5)?,
                indexed_at: row.get(6)?,
            })
        })?;
        let mut files = Vec::new();
        for r in rows {
            files.push(r?);
        }
        Ok(files)
    }

    /// Deletes a file record by repo and path. Cascades to symbols and edges.
    pub fn delete_file(&self, repo: &str, path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM files WHERE repo = ?1 AND path = ?2",
            params![repo, path],
        )?;
        Ok(())
    }

    /// Deletes a file record and its associated data (same as delete_file due to foreign keys).
    pub fn delete_file_cascade(&self, repo: &str, path: &str) -> Result<()> {
        self.delete_file(repo, path)
    }

    /// Deletes all files for a given repository.
    pub fn delete_files_for_repo(&self, repo: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE repo = ?1", params![repo])?;
        Ok(())
    }

    /// Alias for deleting all files for a given repository.
    pub fn delete_repo_files(&self, repo: &str) -> Result<()> {
        self.delete_files_for_repo(repo)
    }

    /// Deletes a repository and all associated files, symbols, and edges (via foreign key cascades).
    /// Returns the counts of removed items, or None if the repository was not found.
    pub fn delete_repo(&self, repo: &str) -> Result<Option<RepoCleanStats>> {
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM repos WHERE id = ?1",
            params![repo],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }

        let files_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE repo = ?1",
            params![repo],
            |r| r.get(0),
        )?;
        let symbols_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols WHERE repo = ?1",
            params![repo],
            |r| r.get(0),
        )?;
        let edges_count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE repo = ?1",
            params![repo],
            |r| r.get(0),
        )?;

        // Deleting from repos cascades to files, symbols, edges; symbols_ad cleans symbols_fts
        self.conn.execute("DELETE FROM repos WHERE id = ?1", params![repo])?;

        Ok(Some(RepoCleanStats {
            repo: repo.to_owned(),
            files_removed: files_count,
            symbols_removed: symbols_count,
            edges_removed: edges_count,
        }))
    }

    /// Cleans all data from the graph database (repos, files, symbols, edges).
    pub fn clean_all(&self) -> Result<CleanAllStats> {
        let repos_count: usize = self.conn.query_row("SELECT COUNT(*) FROM repos", [], |r| r.get(0))?;
        let files_count: usize = self.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let symbols_count: usize = self.conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
        let edges_count: usize = self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;

        self.conn.execute_batch(
            "DELETE FROM repos;
             DELETE FROM files;
             DELETE FROM symbols;
             DELETE FROM edges;"
        )?;

        Ok(CleanAllStats {
            repos_removed: repos_count,
            files_removed: files_count,
            symbols_removed: symbols_count,
            edges_removed: edges_count,
        })
    }
}
