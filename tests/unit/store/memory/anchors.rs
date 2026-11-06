#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::memory_anchor::{Anchor, Candidate};
use crate::store::graph::db::GraphDb;
use crate::store::memory::db::MemoryDb;

struct Fixture {
    _dir: tempfile::TempDir,
    graph: Utf8PathBuf,
    memory: MemoryDb,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    let graph = root.join("graph.db");
    let g = GraphDb::open(graph.as_std_path()).unwrap();
    g.conn()
        .execute_batch(
            "INSERT INTO repos (id, root_path, default_branch, indexed_at) VALUES
               ('ivar', '/i', 'main', 0), ('web', '/w', 'main', 0), ('ivar/feat', '/l', 'f', 0);
             INSERT INTO files (id, repo, path, content_hash, mtime_ns, size_bytes, indexed_at) VALUES
               (1, 'ivar', 'src/a.rs', 'h', 0, 1, 0),
               (2, 'ivar', 'README.md', 'h', 0, 1, 0),
               (3, 'web', 'README.md', 'h', 0, 1, 0),
               (4, 'web', 'src/b.ts', 'h', 0, 1, 0),
               (5, 'ivar/feat', 'src/only_in_layer.rs', 'h', 0, 1, 0);
             INSERT INTO symbols (file_id, repo, name, kind, scope, start_line, start_col, end_line, end_col) VALUES
               (1, 'ivar', 'refresh_locked', 'function', NULL, 1, 0, 2, 0),
               (1, 'ivar', 'open', 'method', 'MemoryDb', 3, 0, 4, 0),
               (4, 'web', 'open', 'function', NULL, 1, 0, 2, 0);",
        )
        .unwrap();
    let memory = MemoryDb::open(root.join("memory.db").as_std_path()).unwrap();
    Fixture {
        _dir: dir,
        graph,
        memory,
    }
}

fn path(repo: Option<&str>, p: &str) -> Candidate {
    Candidate::Path {
        repo: repo.map(str::to_owned),
        path: p.to_owned(),
    }
}

fn sym(scope: Option<&str>, name: &str) -> Candidate {
    Candidate::Symbol {
        scope: scope.map(str::to_owned),
        name: name.to_owned(),
    }
}

fn anchor(repo: &str, p: &str, symbol: Option<&str>) -> Anchor {
    Anchor {
        repo: repo.into(),
        path: p.into(),
        symbol: symbol.map(str::to_owned),
    }
}

fn resolved(f: &mut Fixture, candidates: &[Candidate]) -> Vec<Anchor> {
    assert!(attach_graph(f.memory.conn(), &f.graph));
    let tx = f.memory.conn_mut().transaction().unwrap();
    let anchors = resolve(&tx, candidates).unwrap();
    tx.commit().unwrap();
    detach_graph(f.memory.conn());
    anchors
}

#[test]
fn unique_paths_and_symbols_resolve_against_the_base_layer_only() {
    let mut f = fixture();
    let anchors = resolved(
        &mut f,
        &[
            path(None, "src/a.rs"),
            path(Some("web"), "src/b.ts"),
            path(None, "ivar/src/a.rs"),
            path(None, "src/only_in_layer.rs"),
            sym(None, "refresh_locked"),
            sym(Some("MemoryDb"), "open"),
        ],
    );
    assert_eq!(
        anchors,
        vec![
            anchor("ivar", "src/a.rs", None),
            anchor("ivar", "src/a.rs", Some("open")),
            anchor("ivar", "src/a.rs", Some("refresh_locked")),
            anchor("web", "src/b.ts", None),
        ]
    );
}

#[test]
fn an_ambiguous_path_or_symbol_needs_another_resolved_citation() {
    let mut f = fixture();
    assert!(resolved(&mut f, &[path(None, "README.md"), sym(None, "open")]).is_empty());
    assert_eq!(
        resolved(
            &mut f,
            &[
                path(None, "README.md"),
                sym(None, "open"),
                path(None, "src/b.ts")
            ]
        ),
        vec![
            anchor("web", "README.md", None),
            anchor("web", "src/b.ts", None),
            anchor("web", "src/b.ts", Some("open")),
        ]
    );
}

#[test]
fn stored_anchors_are_read_matched_and_reported_broken() {
    let mut f = fixture();
    f.memory
        .conn()
        .execute_batch(
            "INSERT INTO pages (path, kind, tier, title, size, mtime_ns, content_hash, indexed_at)
         VALUES ('docs/memory/decision/a.md', 'decision', 'semantic', 'A', 1, 1, 'h', 't'),
                ('.ivar/memory/session/s.md', 'session', 'working', 'S', 1, 1, 'h', 't');",
        )
        .unwrap();
    let tx = f.memory.conn_mut().transaction().unwrap();
    replace_anchors(
        &tx,
        "docs/memory/decision/a.md",
        &[
            anchor("ivar", "src/a.rs", Some("refresh_locked")),
            anchor("ivar", "src/gone.rs", None),
        ],
    )
    .unwrap();
    replace_anchors(
        &tx,
        ".ivar/memory/session/s.md",
        &[anchor("ivar", "src/a.rs", None)],
    )
    .unwrap();
    set_pending(&tx, ".ivar/memory/session/s.md", true).unwrap();
    assert_eq!(
        pending_paths(&tx).unwrap(),
        vec![".ivar/memory/session/s.md"]
    );
    tx.commit().unwrap();
    assert_eq!(pending_count(f.memory.conn()).unwrap(), 1);

    assert_eq!(
        match_terms(f.memory.conn(), &["Refresh_Locked".into(), "a".into()]).unwrap()[0],
        ("docs/memory/decision/a.md".to_owned(), 2, 1)
    );
    assert!(attach_graph(f.memory.conn(), &f.graph));
    let views = anchors_for(f.memory.conn(), "docs/memory/decision/a.md", true, 10).unwrap();
    assert_eq!(
        (views[0].path.as_str(), views[0].broken),
        ("src/gone.rs", true)
    );
    assert!(!views[1].broken);
    assert_eq!(
        broken_anchors(f.memory.conn()).unwrap(),
        vec![(
            "docs/memory/decision/a.md".to_owned(),
            anchor("ivar", "src/gone.rs", None)
        )]
    );
    detach_graph(f.memory.conn());

    assert!(
        related_pages(
            f.memory.conn(),
            &[("ivar".into(), "src/other.rs".into())],
            3,
        )
        .unwrap()
        .is_empty(),
        "a symbol name alone never relates a page"
    );

    let related = related_pages(f.memory.conn(), &[("ivar".into(), "src/a.rs".into())], 3).unwrap();
    assert_eq!(related.len(), 1, "session pages are never related memory");
    assert_eq!(
        (related[0].path.as_str(), related[0].anchors),
        ("docs/memory/decision/a.md", 1)
    );
}

#[test]
fn attach_refuses_a_missing_graph() {
    let f = fixture();
    assert!(!attach_graph(
        f.memory.conn(),
        &f.graph.with_file_name("absent.db")
    ));
}

#[test]
fn a_path_anchor_matches_a_query_term_in_any_case() {
    let mut f = fixture();
    f.memory
        .conn()
        .execute_batch(
            "INSERT INTO pages (path, kind, tier, title, size, mtime_ns, content_hash, indexed_at)
             VALUES ('docs/memory/decision/q.md', 'decision', 'semantic', 'Q', 1, 1, 'h', 't');",
        )
        .unwrap();
    let tx = f.memory.conn_mut().transaction().unwrap();
    replace_anchors(
        &tx,
        "docs/memory/decision/q.md",
        &[anchor("ivar", "src/store/quokka.rs", None)],
    )
    .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        match_terms(f.memory.conn(), &["QUOKKA".into()]).unwrap(),
        vec![("docs/memory/decision/q.md".to_owned(), 1, 1)]
    );
}

#[test]
fn a_repo_prefixed_path_is_extracted_whole_and_stored_under_its_repo() {
    assert_eq!(
        crate::domain::memory_anchor::extract_candidates("See `ivar/src/a.rs`."),
        vec![path(None, "ivar/src/a.rs")]
    );
    let mut f = fixture();
    assert_eq!(
        resolved(&mut f, &[path(None, "ivar/src/a.rs")]),
        vec![anchor("ivar", "src/a.rs", None)]
    );
}

#[test]
fn anchored_files_lists_each_durable_page_file_pair_once() {
    let mut f = fixture();
    f.memory
        .conn()
        .execute_batch(
            "INSERT INTO pages (path, kind, tier, title, size, mtime_ns, content_hash, indexed_at) VALUES
               ('docs/memory/decision/a.md', 'decision', 'semantic', 'A', 1, 1, 'h', 't'),
               ('docs/memory/gotcha/b.md', 'gotcha', 'semantic', 'B', 1, 1, 'h', 't'),
               ('.ivar/memory/session/s.md', 'session', 'episodic', 'S', 1, 1, 'h', 't');",
        )
        .unwrap();
    let tx = f.memory.conn_mut().transaction().unwrap();
    replace_anchors(
        &tx,
        "docs/memory/decision/a.md",
        &[
            anchor("ivar", "src/a.rs", None),
            anchor("ivar", "src/a.rs", Some("open")),
        ],
    )
    .unwrap();
    replace_anchors(
        &tx,
        "docs/memory/gotcha/b.md",
        &[anchor("web", "src/b.ts", None)],
    )
    .unwrap();
    replace_anchors(
        &tx,
        ".ivar/memory/session/s.md",
        &[anchor("ivar", "src/a.rs", None)],
    )
    .unwrap();
    tx.commit().unwrap();

    let files = [("ivar".to_owned(), "src/a.rs".to_owned())];

    assert_eq!(
        anchored_files(f.memory.conn(), &files).unwrap(),
        vec![(
            "docs/memory/decision/a.md".to_owned(),
            "ivar".to_owned(),
            "src/a.rs".to_owned()
        )]
    );
}
