#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fs;

use super::*;
use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::memory::db::MemoryDb;

type AnchorRow<'a> = (&'a str, &'a str, Option<&'a str>);

fn graph_with_one_file(root: &Path) -> GraphDb {
    let repo_dir = root.join("api");
    fs::create_dir_all(repo_dir.join("src")).unwrap();
    let content = "export function createSession() {\n  return 1;\n}\n";
    fs::write(repo_dir.join("src/sessions.ts"), content).unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .unwrap();
    let file_id = db
        .upsert_file(
            "api",
            "src/sessions.ts",
            &crate::infra::hash::text(content),
            1,
            content.len() as i64,
        )
        .unwrap();
    db.insert_symbols(&[Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "api".into(),
        name: "createSession".into(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, 3, 1),
        is_exported: true,
        complexity: None,
    }])
    .unwrap();
    db
}

fn memory_with(root: &Path, pages: &[(&str, &str, &str, &[AnchorRow])]) {
    fs::create_dir_all(root.join(".ivar")).unwrap();
    let db = MemoryDb::open(&root.join(".ivar/memory.db")).unwrap();
    for (path, kind, tier, anchors) in pages {
        db.conn()
            .execute(
                "INSERT INTO pages (path, kind, tier, title, size, mtime_ns, content_hash, indexed_at)
                 VALUES (?1, ?2, ?3, ?1, 1, 1, 'h', 't')",
                rusqlite::params![path, kind, tier],
            )
            .unwrap();
        for (repo, file, symbol) in *anchors {
            db.conn()
                .execute(
                    "INSERT INTO page_anchors (page_path, repo, path, symbol) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![path, repo, file, symbol],
                )
                .unwrap();
        }
    }
}

#[test]
fn pages_anchored_to_the_result_are_listed_best_first_and_capped_at_three() {
    let dir = tempfile::tempdir().unwrap();
    let db = graph_with_one_file(dir.path());
    let file: &[AnchorRow] = &[("api", "src/sessions.ts", None)];
    let both: &[AnchorRow] = &[
        ("api", "src/sessions.ts", None),
        ("api", "src/sessions.ts", Some("createSession")),
    ];
    memory_with(
        dir.path(),
        &[
            ("docs/memory/update/001-a.md", "update", "episodic", file),
            ("docs/memory/decision/b.md", "decision", "semantic", file),
            ("docs/memory/gotcha/c.md", "gotcha", "procedural", both),
            ("docs/memory/concept/d.md", "concept", "semantic", file),
            (".ivar/memory/session/s.md", "session", "working", both),
        ],
    );

    let res = explore(&db, dir.path(), "src/sessions.ts", None).unwrap();

    let paths: Vec<&str> = res.related_memory.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "docs/memory/gotcha/c.md",
            "docs/memory/concept/d.md",
            "docs/memory/decision/b.md"
        ]
    );
    assert_eq!(res.related_memory[0].anchors, 2);
}

#[test]
fn a_missing_memory_database_leaves_the_section_empty() {
    let dir = tempfile::tempdir().unwrap();
    let db = graph_with_one_file(dir.path());

    let res = explore(&db, dir.path(), "src/sessions.ts", None).unwrap();

    assert!(res.related_memory.is_empty());
    assert!(
        !dir.path().join(".ivar/memory.db").exists(),
        "explore never creates memory.db"
    );
}

#[test]
fn related_memory_is_in_the_json_only_when_a_page_is_related() {
    let dir = tempfile::tempdir().unwrap();
    let db = graph_with_one_file(dir.path());
    let empty = explore(&db, dir.path(), "src/sessions.ts", None).unwrap();
    assert!(
        serde_json::to_value(&empty)
            .unwrap()
            .get("related_memory")
            .is_none()
    );

    let file: &[AnchorRow] = &[("api", "src/sessions.ts", None)];
    memory_with(
        dir.path(),
        &[("docs/memory/decision/b.md", "decision", "semantic", file)],
    );
    let related = explore(&db, dir.path(), "src/sessions.ts", None).unwrap();

    let json = serde_json::to_value(&related).unwrap();
    assert_eq!(
        json["related_memory"][0]["path"],
        "docs/memory/decision/b.md"
    );
}
