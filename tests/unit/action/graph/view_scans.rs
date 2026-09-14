#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::action::graph::query::{
    find::explore_find, find_symbols, get_callees, get_callers, get_callers_of, get_file_outline,
    get_impact, get_references, get_references_of,
};
use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;

const FILES: usize = 1000;
const SYMBOLS_PER_FILE: usize = 20;
/// A lookup through an index costs a few thousand VM steps; walking any of the
/// 20k symbol or edge rows costs well over a hundred thousand.
const MAX_VM_STEPS: u64 = 60_000;

fn symbol(file_id: i64, repo: &str, name: String, line: usize) -> Symbol {
    Symbol {
        id: None,
        file_id: Some(file_id),
        repo: repo.to_owned(),
        name,
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(line, 1, line, 40),
        is_exported: true,
        complexity: None,
    }
}

fn edge(
    repo: &str,
    file_id: i64,
    from: Option<i64>,
    to: i64,
    name: String,
    kind: EdgeKind,
) -> Edge {
    Edge {
        id: None,
        repo: repo.to_owned(),
        file_id: Some(file_id),
        from_symbol_id: from,
        to_symbol_id: Some(to),
        to_name: Some(name),
        kind,
        provenance: Provenance::Extracted,
        line: 1,
        col: 1,
        confidence: 1.0,
    }
}

fn seeded_db() -> (GraphDb, i64) {
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();
    db.insert_repo("app", "/app", "main", None).unwrap();
    let mut previous: Option<(i64, String)> = None;
    let mut target = 0;
    for file in 0..FILES {
        let file_id = db
            .upsert_file("app", &format!("src/m{file}/code.rs"), "h", 1, 1)
            .unwrap();
        let symbols: Vec<Symbol> = (0..SYMBOLS_PER_FILE)
            .map(|i| symbol(file_id, "app", format!("f{file}_s{i}"), i + 1))
            .collect();
        let ids = db.insert_symbols(&symbols).unwrap();
        let mut edges = Vec::new();
        for (id, sym) in ids.iter().zip(&symbols) {
            if let Some((prev_id, _)) = &previous {
                edges.push(edge(
                    "app",
                    file_id,
                    Some(*prev_id),
                    *id,
                    sym.name.clone(),
                    EdgeKind::Calls,
                ));
            }
            edges.push(edge(
                "app",
                file_id,
                None,
                *id,
                sym.name.clone(),
                EdgeKind::References,
            ));
            previous = Some((*id, sym.name.clone()));
        }
        db.insert_edges(&edges).unwrap();
        if file == FILES / 2 {
            target = ids[SYMBOLS_PER_FILE / 2];
        }
    }
    (db, target)
}

fn configure_one_layer(db: &GraphDb) {
    let layer_id = db
        .ensure_layer_record("feat", "app", "/feat/app", "c1")
        .unwrap();
    let layer_repo = format!("app/{layer_id}");
    db.insert_repo(&layer_repo, "/feat/app", "feat", None)
        .unwrap();
    let file_id = db
        .upsert_file(&layer_repo, "src/m0/code.rs", "h2", 2, 2)
        .unwrap();
    db.insert_symbols(&[symbol(file_id, &layer_repo, "f0_s0".to_owned(), 1)])
        .unwrap();
    db.set_layer_tombstones(layer_id, &["src/m1/code.rs"])
        .unwrap();
    db.configure_session_mode(&[("app", &layer_repo)]).unwrap();
}

fn vm_steps(db: &GraphDb, query: impl FnOnce()) -> u64 {
    let steps = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&steps);
    db.conn().progress_handler(
        1,
        Some(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            false
        }),
    );
    query();
    db.conn().progress_handler(1, None::<fn() -> bool>);
    steps.load(Ordering::Relaxed)
}

fn heavy_queries(db: &GraphDb, target: i64) -> Vec<(&'static str, u64)> {
    let name = format!("f{}_s{}", FILES / 2, SYMBOLS_PER_FILE / 2);
    let path = format!("src/m{}/code.rs", FILES / 2);
    vec![
        (
            "get_callers",
            vm_steps(db, || {
                assert!(!get_callers(db, &name, None, true, 0.0).unwrap().is_empty());
            }),
        ),
        (
            "get_callers_of",
            vm_steps(db, || {
                assert!(!get_callers_of(db, target, 0.0).unwrap().is_empty());
            }),
        ),
        (
            "get_references",
            vm_steps(db, || {
                assert!(!get_references(db, &name, None).unwrap().is_empty());
            }),
        ),
        (
            "get_references_of",
            vm_steps(db, || {
                assert!(!get_references_of(db, target).unwrap().is_empty());
            }),
        ),
        (
            "get_callees",
            vm_steps(db, || {
                assert!(!get_callees(db, target).unwrap().is_empty());
            }),
        ),
        (
            "get_impact",
            vm_steps(db, || {
                assert!(get_impact(db, target, 3).unwrap().total_affected > 0);
            }),
        ),
        (
            "find_symbols",
            vm_steps(db, || {
                assert!(!find_symbols(db, &name, None, 10).unwrap().is_empty());
            }),
        ),
        (
            "explore_find",
            vm_steps(db, || {
                assert!(!explore_find(db, &name, None, 4).unwrap().symbols.is_empty());
            }),
        ),
        (
            "get_file_outline",
            vm_steps(db, || {
                assert!(
                    !get_file_outline(db, "app", &path)
                        .unwrap()
                        .symbols
                        .is_empty()
                );
            }),
        ),
    ]
}

fn assert_no_table_walks(steps: Vec<(&'static str, u64)>) {
    let walks: Vec<_> = steps
        .into_iter()
        .filter(|(_, count)| *count > MAX_VM_STEPS)
        .collect();
    assert!(walks.is_empty(), "queries walking whole tables: {walks:?}");
}

#[test]
fn base_view_queries_seek_indexes() {
    let (db, target) = seeded_db();
    assert_no_table_walks(heavy_queries(&db, target));
}

#[test]
fn session_view_queries_seek_indexes() {
    let (db, target) = seeded_db();
    configure_one_layer(&db);
    assert_no_table_walks(heavy_queries(&db, target));
}

#[test]
fn edges_into_a_shadowed_symbol_resolve_by_name_in_the_session() {
    let (db, _) = seeded_db();
    let id_of = |name: &str, repo: &str| -> i64 {
        db.conn()
            .query_row(
                "SELECT id FROM symbols WHERE name = ?1 AND repo = ?2",
                [name, repo],
                |row| row.get(0),
            )
            .unwrap()
    };
    let caller = id_of("f5_s0", "app");
    let file_id: i64 = db
        .conn()
        .query_row(
            "SELECT file_id FROM symbols WHERE id = ?1",
            [caller],
            |row| row.get(0),
        )
        .unwrap();
    db.insert_edges(&[edge(
        "app",
        file_id,
        Some(caller),
        id_of("f0_s0", "app"),
        "f0_s0".to_owned(),
        EdgeKind::Calls,
    )])
    .unwrap();
    configure_one_layer(&db);

    let callers = get_callers(&db, "f0_s0", None, true, 0.0).unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].caller.name, "f5_s0");
    let layer_symbol = id_of("f0_s0", "app/1");
    let callers_of = get_callers_of(&db, layer_symbol, 0.0).unwrap();
    assert_eq!(callers_of.len(), 1);
    assert_eq!(callers_of[0].caller.name, "f5_s0");
}
