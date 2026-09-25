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
        file_matches: Vec::new(),
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

fn index_session_files(workspace: &Path, count: usize) -> GraphDb {
    let repo_root = workspace.join("services/api");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo dir");
    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("api", repo_root.to_str().expect("utf8 path"), "main", None)
        .expect("insert repo");
    for n in 0..count {
        let path = format!("src/session{n}.ts");
        let content = format!("export function session{n}() {{\n  return {n};\n}}\n");
        std::fs::write(repo_root.join(&path), &content).expect("write source");
        let file_id = db
            .upsert_file("api", &path, &crate::infra::hash::text(&content), 1, 1)
            .expect("upsert file");
        db.insert_symbols(&[Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "api".into(),
            name: format!("session{n}"),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 3, 1),
            is_exported: true,
            complexity: None,
        }])
        .expect("insert symbols");
    }
    db
}

fn next_call_args(answer: &str) -> serde_json::Value {
    let line = answer
        .lines()
        .find(|line| line.starts_with("Next: call `graph_explore` with "))
        .unwrap_or_else(|| panic!("no next call in: {answer}"));
    let json = &line[line.find('{').expect("args start")..=line.rfind('}').expect("args end")];
    serde_json::from_str(json).expect("parseable args")
}

#[test]
fn an_answer_that_leaves_files_out_hands_over_the_explore_call_that_returns_them() {
    use crate::action::graph::{explore, narrate};

    let workspace = tempdir().expect("workspace");
    let db = index_session_files(workspace.path(), 8);
    let mut paths = WorkspacePaths::new(Some(workspace.path().to_path_buf()));

    let mut res = explore::explore(&db, workspace.path(), "session", None).expect("explore");
    paths.rewrite_explore(&db, &mut res);
    let answer = narrate::narrate_explore(&res);

    let shown: Vec<&str> = res.sources.iter().map(|s| s.file_path.as_str()).collect();
    let mut omitted: Vec<String> = (0..8)
        .map(|n| format!("services/api/src/session{n}.ts"))
        .filter(|path| !shown.contains(&path.as_str()))
        .collect();
    assert!(!omitted.is_empty(), "the intent budget leaves files out");
    let args = next_call_args(&answer);
    let mut suggested: Vec<String> = args["paths"]
        .as_array()
        .expect("paths array")
        .iter()
        .map(|p| p.as_str().expect("path").to_owned())
        .collect();
    suggested.sort();
    omitted.sort();
    assert_eq!(suggested, omitted);

    let mut follow_up =
        explore::explore_files(&db, workspace.path(), &suggested.join(" "), None).expect("paths");
    paths.rewrite_explore(&db, &mut follow_up);
    let answer = narrate::narrate_requested_files(&follow_up);
    for path in &suggested {
        let source = follow_up
            .sources
            .iter()
            .find(|s| &s.file_path == path)
            .unwrap_or_else(|| panic!("{path} missing from the follow-up"));
        assert!(source.excerpts[0].code.contains("export function session"));
        assert!(answer.contains(&format!("`{path}`")));
    }
    assert!(!answer.contains("Next: call"));
}
