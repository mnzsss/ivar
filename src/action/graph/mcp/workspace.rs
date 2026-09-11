//! Rewrites repo-relative paths in Markdown graph answers to the paths an agent
//! sees in its workspace, so it can pass them straight to the next call.
//!
//! The index stores `src/routes/auth.ts` for a repo mounted at `services/api`.
//! An agent handed the repo-relative path globbed the workspace to find the file
//! before it could use it; in tokens3 that was 2 to 7 globs per discovery run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::action::graph::query::{
    CalleeInfo, CallerInfo, FileOutline, ImpactResult, ReferenceSite, SymbolLocation,
};
use crate::domain::graph::ExploreResult;
use crate::store::graph::db::GraphDb;

/// Workspace prefixes per repo, computed once per answer.
#[derive(Debug)]
pub struct WorkspacePaths {
    workspace: Option<PathBuf>,
    prefixes: HashMap<String, Option<String>>,
}

impl WorkspacePaths {
    pub fn new(workspace: Option<PathBuf>) -> Self {
        Self {
            workspace,
            prefixes: HashMap::new(),
        }
    }

    /// The server's working directory is the agent's workspace: hosts start MCP
    /// servers there.
    pub fn from_current_dir() -> Self {
        Self::new(std::env::current_dir().ok())
    }

    pub fn rewrite_explore(&mut self, db: &GraphDb, res: &mut ExploreResult) {
        for snippet in &mut res.primary_symbols {
            self.rewrite(db, &snippet.symbol.repo, &mut snippet.file_path);
        }
        for source in &mut res.sources {
            self.rewrite(db, &source.repo, &mut source.file_path);
        }
        for relation in res.direct_relations.iter_mut().chain(&mut res.entry_points) {
            self.rewrite(db, &relation.source.repo, &mut relation.source.file_path);
            self.rewrite(db, &relation.target.repo, &mut relation.target.file_path);
        }
        for consumer in &mut res.transitive_consumers {
            self.rewrite(db, &consumer.repo, &mut consumer.file_path);
        }
        for file in &mut res.not_shown {
            self.rewrite(db, &file.repo, &mut file.file_path);
        }
    }

    pub fn rewrite_callers(
        &mut self,
        db: &GraphDb,
        definitions: &mut [SymbolLocation],
        callers: &mut [CallerInfo],
        references: &mut [ReferenceSite],
    ) {
        for definition in definitions {
            self.rewrite(db, &definition.symbol.repo, &mut definition.file_path);
        }
        for caller in callers {
            self.rewrite(db, &caller.caller.repo, &mut caller.caller_file_path);
        }
        for reference in references {
            self.rewrite(db, &reference.repo, &mut reference.file_path);
        }
    }

    pub fn rewrite_callees(&mut self, db: &GraphDb, callees: &mut [CalleeInfo]) {
        for callee in callees {
            if let (Some(symbol), Some(path)) =
                (&callee.callee_symbol, &mut callee.callee_file_path)
            {
                self.rewrite(db, &symbol.repo, path);
            }
        }
    }

    pub fn rewrite_impact(&mut self, db: &GraphDb, impact: &mut ImpactResult) {
        for item in &mut impact.affected_symbols {
            self.rewrite(db, &item.symbol.repo, &mut item.file_path);
        }
    }

    pub fn rewrite_outline(&mut self, db: &GraphDb, outline: &mut FileOutline) {
        self.rewrite(db, &outline.repo, &mut outline.file_path);
    }

    fn rewrite(&mut self, db: &GraphDb, repo: &str, path: &mut String) {
        if path.is_empty() {
            return;
        }
        if let Some(prefix) = self.prefix(db, repo).filter(|prefix| !prefix.is_empty()) {
            *path = format!("{prefix}/{path}");
        }
    }

    fn prefix(&mut self, db: &GraphDb, repo: &str) -> Option<String> {
        if let Some(prefix) = self.prefixes.get(repo) {
            return prefix.clone();
        }
        // A path is only a display aid, so a failed lookup keeps the repo-relative path.
        let root = db
            .get_visible_repo(repo)
            .ok()
            .flatten()
            .map(|row| PathBuf::from(row.root_path));
        let prefix = self
            .workspace
            .as_deref()
            .zip(root)
            .and_then(|(workspace, root)| workspace_prefix(workspace, &root));
        self.prefixes.insert(repo.to_owned(), prefix.clone());
        prefix
    }
}

/// Where a repo root appears in the workspace: its relative path when it lies
/// inside (`services/api`), or the name of a top-level entry that links to it,
/// as a session view does (`ivar`). `None` when the workspace cannot reach it.
pub fn workspace_prefix(workspace: &Path, repo_root: &Path) -> Option<String> {
    let workspace = workspace.canonicalize().ok()?;
    let root = repo_root.canonicalize().ok()?;
    if let Ok(relative) = root.strip_prefix(&workspace) {
        return Some(relative.to_string_lossy().into_owned());
    }
    std::fs::read_dir(&workspace)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.path().canonicalize().is_ok_and(|path| path == root))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/workspace.rs"]
mod tests;
