#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};
use std::io::Cursor;
use tempfile::tempdir;

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;
fn setup_test_mcp_db() -> (GraphDb, tempfile::TempDir) {
    let temp = tempdir().expect("tempdir");
    let hall_root = temp.path();
    let repo_dir = hall_root.join("my-repo");
    std::fs::create_dir_all(repo_dir.join("src")).expect("mkdir src");
    std::fs::create_dir_all(repo_dir.join("tests")).expect("mkdir tests");

    let main_rs = "pub fn execute() {\n    helper();\n}\npub fn helper() {}\n";
    let test_rs = "#[test]\nfn test_exec() {\n    execute();\n}\n";

    std::fs::write(repo_dir.join("src/main.rs"), main_rs).expect("write main.rs");
    std::fs::write(repo_dir.join("tests/exec_test.rs"), test_rs).expect("write test.rs");

    let db = GraphDb::open_in_memory().expect("open db");
    db.insert_repo("my-repo", repo_dir.to_str().unwrap(), "main", None)
        .expect("insert repo");

    let f1 = db
        .upsert_file("my-repo", "src/main.rs", "h1", 100, 100)
        .expect("upsert f1");
    let f2 = db
        .upsert_file("my-repo", "tests/exec_test.rs", "h2", 100, 100)
        .expect("upsert f2");

    let s1 = Symbol {
        id: None,
        file_id: Some(f1),
        repo: "my-repo".to_owned(),
        name: "execute".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn execute()".to_owned()),
        docstring: Some("Executes main flow.".to_owned()),
        span: Span::new(1, 1, 3, 1),
        is_exported: true,
        complexity: None,
    };
    let s2 = Symbol {
        id: None,
        file_id: Some(f1),
        repo: "my-repo".to_owned(),
        name: "helper".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("pub fn helper()".to_owned()),
        docstring: None,
        span: Span::new(4, 1, 4, 18),
        is_exported: true,
        complexity: None,
    };
    let s3 = Symbol {
        id: None,
        file_id: Some(f2),
        repo: "my-repo".to_owned(),
        name: "test_exec".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: Some("fn test_exec()".to_owned()),
        docstring: None,
        span: Span::new(2, 1, 4, 1),
        is_exported: false,
        complexity: None,
    };

    let ids = db.insert_symbols(&[s1, s2, s3]).expect("insert symbols");
    let s1_id = ids[0];
    let s2_id = ids[1];
    let s3_id = ids[2];

    // execute calls helper
    db.insert_edges(&[
        crate::domain::graph::Edge {
            id: None,
            repo: "my-repo".to_owned(),
            file_id: Some(f1),
            from_symbol_id: Some(s1_id),
            to_symbol_id: Some(s2_id),
            to_name: Some("helper".to_owned()),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            line: 2,
            col: 5,
            provenance: Provenance::Extracted,
        },
        // test_exec calls execute
        crate::domain::graph::Edge {
            id: None,
            repo: "my-repo".to_owned(),
            file_id: Some(f2),
            from_symbol_id: Some(s3_id),
            to_symbol_id: Some(s1_id),
            to_name: Some("execute".to_owned()),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            line: 3,
            col: 5,
            provenance: Provenance::Extracted,
        },
    ])
    .expect("insert edges");

    (db, temp)
}

fn index_file(db: &GraphDb, root: &std::path::Path, path: &str, content: &str, symbol: &str) {
    std::fs::write(root.join("my-repo").join(path), content).expect("write source");
    let file_id = db
        .upsert_file(
            "my-repo",
            path,
            &crate::infra::hash::text(content),
            100,
            100,
        )
        .expect("upsert file");
    db.insert_symbols(&[Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "my-repo".to_owned(),
        name: symbol.to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, 3, 1),
        is_exported: true,
        complexity: None,
    }])
    .expect("insert symbol");
}

fn long_file() -> String {
    let body: String = (4..=300).map(|n| format!("// line {n}\n")).collect();
    format!("pub fn bulky() {{\n}}\n\n{body}")
}

fn call_tool(db: &GraphDb, root: &std::path::Path, name: &str, arguments: Value) -> (String, bool) {
    let input = format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    );
    let mut output = Vec::new();
    run_mcp_server(db, Some(root), Cursor::new(input), &mut output, |_| {
        Ok(json!({"status": "ok"}))
    })
    .expect("run server");
    let resp: Value =
        serde_json::from_str(String::from_utf8(output).expect("utf8").trim()).expect("parse resp");
    (
        resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content")
            .to_owned(),
        resp["result"]["isError"].as_bool().unwrap_or(false),
    )
}

fn call_tool_at(db: &GraphDb, root: &std::path::Path, cwd: &camino::Utf8Path) -> (String, bool) {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "graph_explore", "arguments": { "query": "execute" } }
    });
    let resp = handle_json_rpc_at(
        db,
        Some(root),
        cwd,
        ToolSurface::All,
        &req,
        &mut |_| Ok(json!({"status": "ok"})),
        None,
    )
    .expect("response");
    (
        resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content")
            .to_owned(),
        resp["result"]["isError"].as_bool().unwrap_or(false),
    )
}

fn fanout_fixture() -> (GraphDb, tempfile::TempDir) {
    let (db, temp) = setup_test_mcp_db();
    let root = temp.path();
    let hub_code = "pub fn fanout_root() {}\n";
    std::fs::write(root.join("my-repo/src/fanout.rs"), hub_code).unwrap();
    let hub_file = db
        .upsert_file(
            "my-repo",
            "src/fanout.rs",
            &crate::infra::hash::text(hub_code),
            100,
            100,
        )
        .unwrap();
    let make_symbol = |repo: &str, file_id: i64, name: String, end: usize| Symbol {
        id: None,
        file_id: Some(file_id),
        repo: repo.into(),
        name,
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, end, 1),
        is_exported: true,
        complexity: None,
    };
    let hub = db
        .insert_symbols(&[make_symbol("my-repo", hub_file, "fanout_root".into(), 1)])
        .unwrap()[0];
    let consumers = root.join("consumer-repo");
    std::fs::create_dir_all(consumers.join("src")).unwrap();
    db.insert_repo("consumer-repo", consumers.to_str().unwrap(), "main", None)
        .unwrap();
    for n in 0..400 {
        let name = format!("caller_{n:03}");
        let path = format!("src/caller_{n:03}.rs");
        let source =
            format!("pub fn {name}() {{\n    fanout_root();\n    // omitted_evidence_{n:03}\n}}\n");
        std::fs::write(consumers.join(&path), &source).unwrap();
        let file = db
            .upsert_file(
                "consumer-repo",
                &path,
                &crate::infra::hash::text(&source),
                100,
                100,
            )
            .unwrap();
        let caller = db
            .insert_symbols(&[make_symbol("consumer-repo", file, name, 4)])
            .unwrap()[0];
        db.insert_edges(&[crate::domain::graph::Edge {
            id: None,
            repo: "consumer-repo".into(),
            file_id: Some(file),
            from_symbol_id: Some(caller),
            to_symbol_id: Some(hub),
            to_name: Some("fanout_root".into()),
            kind: EdgeKind::Calls,
            confidence: 1.0,
            line: 2,
            col: 5,
            provenance: Provenance::Extracted,
        }])
        .unwrap();
    }
    (db, temp)
}

fn git_in(cwd: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A hall at `temp` declaring `core`, checked out at `repos/core/main` with one commit.
fn hall_with_core_checkout(temp: &std::path::Path) -> (camino::Utf8PathBuf, String) {
    use crate::domain::name::{BranchName, RepoName};
    use crate::store::layout::Layout;

    let root = camino::Utf8PathBuf::from_path_buf(temp.to_path_buf()).unwrap();
    std::fs::write(
        root.join("ivar.json"),
        r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[{"default_branch":"main","name":"core","url":"https://example.com/core.git"}],"version":1}"#,
    )
    .unwrap();
    let layout = Layout::at(root.clone());
    let checkout = layout.repo_worktree(
        &RepoName::new("core").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    std::fs::create_dir_all(&checkout).unwrap();
    git_in(checkout.as_std_path(), &["init", "-b", "main"]);
    std::fs::write(checkout.join("a.txt"), "a").unwrap();
    git_in(checkout.as_std_path(), &["add", "."]);
    git_in(checkout.as_std_path(), &["commit", "-m", "one"]);
    let head = git_in(checkout.as_std_path(), &["rev-parse", "HEAD"]);
    (checkout, head)
}

fn explore_content(db: &GraphDb, root: &std::path::Path, cwd: &camino::Utf8Path) -> Vec<String> {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "graph_explore", "arguments": { "query": "execute", "format": "json" } }
    });
    let resp = handle_json_rpc_at(
        db,
        Some(root),
        cwd,
        ToolSurface::All,
        &req,
        &mut |_| Ok(json!({"status": "ok"})),
        None,
    )
    .expect("response");
    resp["result"]["content"]
        .as_array()
        .expect("content blocks")
        .iter()
        .map(|block| block["text"].as_str().unwrap().to_owned())
        .collect()
}

#[path = "mcp/root/compat.rs"]
mod compat;
#[path = "mcp/root/fanout.rs"]
mod fanout;
#[path = "mcp/root/memory_tools.rs"]
mod memory_tools;
#[path = "mcp/root/surface.rs"]
mod surface;
#[path = "mcp/root/tool_calls.rs"]
mod tool_calls;
#[path = "mcp/root/usage.rs"]
mod usage;
