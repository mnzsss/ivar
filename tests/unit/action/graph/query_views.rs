use crate::action::graph::query::{find_symbols, get_file_outline};
use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;

#[test]
fn test_query_reads_reflect_session_layer_and_hide_shadowed_symbols() {
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();

    // 1. Seed base repo "backend"
    db.insert_repo("backend", "/base/backend", "main", Some("c1")).unwrap();
    let f1 = db.upsert_file("backend", "src/auth.rs", "h1", 100, 100).unwrap();
    db.insert_symbols(&[
        Symbol {
            id: None,
            file_id: Some(f1),
            repo: "backend".to_string(),
            name: "old_login".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn old_login()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: Some(1),
        },
    ]).unwrap();

    // 2. Seed layer repo "backend/1" with renamed function "login_v2"
    db.insert_repo("backend/1", "/feat/backend", "feat", Some("c2")).unwrap();
    let f2 = db.upsert_file("backend/1", "src/auth.rs", "h2", 200, 200).unwrap();
    db.insert_symbols(&[
        Symbol {
            id: None,
            file_id: Some(f2),
            repo: "backend/1".to_string(),
            name: "login_v2".to_string(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: Some("pub fn login_v2()".to_string()),
            docstring: None,
            span: Span::new(1, 1, 5, 1),
            is_exported: true,
            complexity: Some(1),
        },
    ]).unwrap();

    // 3. Configure session mode
    db.configure_session_mode(&[("backend", "backend/1")]).unwrap();

    let base_syms = find_symbols(&db, "old_login", Some("backend"), 10).unwrap();
    assert_eq!(base_syms.len(), 0, "Base symbol 'old_login' must be shadowed");
    let feat_syms = find_symbols(&db, "login_v2", Some("backend"), 10).unwrap();
    assert_eq!(feat_syms.len(), 1, "Layer symbol 'login_v2' must be found");
    assert_eq!(feat_syms[0].symbol.repo, "backend", "Symbol repo must be projected as canonical name");

    let outline = get_file_outline(&db, "backend", "src/auth.rs").unwrap();
    assert_eq!(outline.symbols.len(), 1);
    assert_eq!(outline.symbols[0].name, "login_v2");
}
