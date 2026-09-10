#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use tempfile::tempdir;

use crate::domain::graph::{FileMention, SourceFile, Span, Symbol, SymbolKind, SymbolSnippet};

#[test]
fn a_repo_inside_the_workspace_takes_its_relative_directory() {
    let workspace = tempdir().expect("workspace");
    let repo = workspace.path().join("services/api");
    std::fs::create_dir_all(&repo).expect("create repo dir");

    assert_eq!(
        workspace_prefix(workspace.path(), &repo).as_deref(),
        Some("services/api")
    );
    assert_eq!(
        workspace_prefix(workspace.path(), workspace.path()).as_deref(),
        Some("")
    );
}

#[cfg(unix)]
#[test]
fn a_repo_linked_into_the_workspace_takes_the_link_name() {
    let workspace = tempdir().expect("workspace");
    let worktree = tempdir().expect("worktree");
    std::os::unix::fs::symlink(worktree.path(), workspace.path().join("ivar")).expect("symlink");

    assert_eq!(
        workspace_prefix(workspace.path(), worktree.path()).as_deref(),
        Some("ivar")
    );
}

#[test]
fn a_repo_the_workspace_cannot_reach_keeps_repo_relative_paths() {
    let workspace = tempdir().expect("workspace");
    let elsewhere = tempdir().expect("elsewhere");

    assert_eq!(workspace_prefix(workspace.path(), elsewhere.path()), None);
}

#[test]
fn explore_answers_name_files_the_way_the_agent_sees_them() {
    let workspace = tempdir().expect("workspace");
    let repo_root = workspace.path().join("services/api");
    std::fs::create_dir_all(&repo_root).expect("create repo dir");
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("api", repo_root.to_str().expect("utf8 path"), "main", None)
        .expect("insert repo");

    let mut res = ExploreResult {
        query: "auth".into(),
        primary_symbols: vec![SymbolSnippet {
            symbol: Symbol {
                id: Some(1),
                file_id: Some(1),
                repo: "api".into(),
                name: "authRoutes".into(),
                kind: SymbolKind::Const,
                scope: None,
                signature: None,
                docstring: None,
                span: Span::new(6, 1, 30, 2),
                is_exported: true,
                complexity: None,
            },
            file_path: "src/routes/auth.ts".into(),
            code: String::new(),
            start_line: 6,
            end_line: 30,
        }],
        call_flows: Vec::new(),
        impact_summary: None,
        direct_relations: Vec::new(),
        entry_points: Vec::new(),
        transitive_consumers: Vec::new(),
        sources: vec![SourceFile {
            repo: "api".into(),
            file_path: "src/routes/auth.ts".into(),
            line_count: 30,
            excerpts: Vec::new(),
            changed_since_index: false,
        }],
        flows: Vec::new(),
        not_shown: vec![FileMention {
            repo: "api".into(),
            file_path: "src/routes/admin.ts".into(),
            symbols: Vec::new(),
        }],
    };

    WorkspacePaths::new(Some(workspace.path().to_path_buf())).rewrite_explore(&db, &mut res);

    assert_eq!(
        res.primary_symbols[0].file_path,
        "services/api/src/routes/auth.ts"
    );
    assert_eq!(res.sources[0].file_path, res.primary_symbols[0].file_path);
    assert_eq!(
        res.not_shown[0].file_path,
        "services/api/src/routes/admin.ts"
    );
}
