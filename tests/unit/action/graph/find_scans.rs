#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::rank::PATH_TIER_SQL;
use super::search::{NAME_PREFIX_MATCH, prefix_casings};
use super::{explore_find, find_symbols};
use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;

const NAME_STEMS: &[&str] = &[
    "getUser",
    "GetUser",
    "get_user",
    "GETTER",
    "getter",
    "gEtMixed",
    "userService",
    "UserService",
    "USER_LIMIT",
    "parseConfig",
    "ParseError",
    "parse",
    "Parser",
    "handler",
    "HandleRequest",
    "auth_token",
    "AuthToken",
    "configLoader",
];
const PATHS: &[&str] = &[
    "src/users/mod.rs",
    "src/auth/user.rs",
    "src/config.rs",
    "src/Parsers/json.rs",
    "src/handlers/http.rs",
    "lib/misc.rs",
    "tests/user_test.rs",
];

fn seeded_db(files_per_path: usize) -> GraphDb {
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();
    db.insert_repo("app", "/app", "main", None).unwrap();
    for copy in 0..files_per_path {
        for path in PATHS {
            let path = format!("{copy}/{path}");
            let file_id = db.upsert_file("app", &path, "h", 1, 1).unwrap();
            let symbols: Vec<Symbol> = NAME_STEMS
                .iter()
                .enumerate()
                .map(|(line, stem)| Symbol {
                    id: None,
                    file_id: Some(file_id),
                    repo: "app".to_owned(),
                    name: if copy == 0 {
                        (*stem).to_owned()
                    } else {
                        format!("{stem}{copy}")
                    },
                    kind: SymbolKind::Fn,
                    scope: None,
                    signature: None,
                    docstring: None,
                    span: Span::new(line + 1, 1, line + 2, 1),
                    is_exported: true,
                    complexity: None,
                })
                .collect();
            db.insert_symbols(&symbols).unwrap();
        }
    }
    db
}

fn find_snapshot(db: &GraphDb, term: &str) -> Vec<String> {
    find_symbols(db, term, None, 12)
        .unwrap()
        .into_iter()
        .map(|loc| format!("{}@{}", loc.symbol.name, loc.file_path))
        .collect()
}

fn explore_snapshot(db: &GraphDb, term: &str) -> Vec<String> {
    let found = explore_find(db, term, None, 4).unwrap();
    found
        .symbols
        .into_iter()
        .map(|loc| format!("{}@{}", loc.symbol.name, loc.file_path))
        .chain(found.not_shown.into_iter().map(|m| m.file_path))
        .collect()
}

#[test]
fn find_get_matches_the_like_scan_results() {
    let db = seeded_db(3);
    assert_eq!(
        find_snapshot(&db, "get"),
        [
            "GETTER@0/src/users/mod.rs",
            "getter@0/src/users/mod.rs",
            "GETTER@0/src/auth/user.rs",
            "getter@0/src/auth/user.rs",
            "GETTER@0/src/config.rs",
            "getter@0/src/config.rs",
            "GETTER@0/src/Parsers/json.rs",
            "getter@0/src/Parsers/json.rs",
            "GETTER@0/src/handlers/http.rs",
            "getter@0/src/handlers/http.rs",
            "GETTER@0/lib/misc.rs",
            "getter@0/lib/misc.rs",
        ]
    );
}

#[test]
fn explore_get_prefix_skips_mixed_casings() {
    let db = seeded_db(3);
    assert_eq!(
        explore_snapshot(&db, "get"),
        [
            "getUser@0/src/Parsers/json.rs",
            "GetUser@0/src/Parsers/json.rs",
            "get_user@0/src/Parsers/json.rs",
            "GETTER@0/src/Parsers/json.rs",
            "getter@0/src/Parsers/json.rs",
            "getUser@0/src/auth/user.rs",
            "GetUser@0/src/auth/user.rs",
            "get_user@0/src/auth/user.rs",
            "GETTER@0/src/auth/user.rs",
            "getter@0/src/auth/user.rs",
            "getUser@0/src/config.rs",
            "GetUser@0/src/config.rs",
            "get_user@0/src/config.rs",
            "GETTER@0/src/config.rs",
            "getter@0/src/config.rs",
            "getUser@0/src/users/mod.rs",
            "GetUser@0/src/users/mod.rs",
            "get_user@0/src/users/mod.rs",
            "GETTER@0/src/users/mod.rs",
            "getter@0/src/users/mod.rs",
            "0/lib/misc.rs",
            "0/src/handlers/http.rs",
            "1/src/auth/user.rs",
            "1/src/config.rs",
            "1/src/users/mod.rs",
            "0/tests/user_test.rs",
            "1/lib/misc.rs",
            "1/src/Parsers/json.rs",
            "1/src/handlers/http.rs",
            "2/src/auth/user.rs",
            "2/src/users/mod.rs",
            "2/src/config.rs",
        ]
    );
}

#[test]
fn find_user_matches_the_like_scan_results() {
    let db = seeded_db(3);
    assert_eq!(
        find_snapshot(&db, "user"),
        [
            "USER_LIMIT@0/src/users/mod.rs",
            "USER_LIMIT@0/src/auth/user.rs",
            "USER_LIMIT@0/src/config.rs",
            "USER_LIMIT@0/src/Parsers/json.rs",
            "USER_LIMIT@0/src/handlers/http.rs",
            "USER_LIMIT@0/lib/misc.rs",
            "USER_LIMIT@0/tests/user_test.rs",
            "userService@0/src/users/mod.rs",
            "UserService@0/src/users/mod.rs",
            "userService@0/src/auth/user.rs",
            "UserService@0/src/auth/user.rs",
            "userService@0/src/config.rs",
        ]
    );
}

#[test]
fn explore_user_matches_the_like_scan_results() {
    let db = seeded_db(3);
    assert_eq!(
        explore_snapshot(&db, "user"),
        [
            "getUser@0/src/auth/user.rs",
            "GetUser@0/src/auth/user.rs",
            "get_user@0/src/auth/user.rs",
            "userService@0/src/auth/user.rs",
            "UserService@0/src/auth/user.rs",
            "USER_LIMIT@0/src/auth/user.rs",
            "getUser@0/src/users/mod.rs",
            "GetUser@0/src/users/mod.rs",
            "get_user@0/src/users/mod.rs",
            "userService@0/src/users/mod.rs",
            "UserService@0/src/users/mod.rs",
            "USER_LIMIT@0/src/users/mod.rs",
            "getUser1@1/src/auth/user.rs",
            "GetUser1@1/src/auth/user.rs",
            "get_user1@1/src/auth/user.rs",
            "userService1@1/src/auth/user.rs",
            "UserService1@1/src/auth/user.rs",
            "USER_LIMIT1@1/src/auth/user.rs",
            "getUser1@1/src/users/mod.rs",
            "GetUser1@1/src/users/mod.rs",
            "get_user1@1/src/users/mod.rs",
            "userService1@1/src/users/mod.rs",
            "UserService1@1/src/users/mod.rs",
            "USER_LIMIT1@1/src/users/mod.rs",
            "2/src/auth/user.rs",
            "2/src/users/mod.rs",
            "0/src/Parsers/json.rs",
            "0/src/config.rs",
        ]
    );
}

#[test]
fn find_parse_matches_the_like_scan_results() {
    let db = seeded_db(3);
    assert_eq!(
        find_snapshot(&db, "parse"),
        [
            "parse@0/src/users/mod.rs",
            "parse@0/src/auth/user.rs",
            "parse@0/src/config.rs",
            "parse@0/src/Parsers/json.rs",
            "parse@0/src/handlers/http.rs",
            "parse@0/lib/misc.rs",
            "parse@0/tests/user_test.rs",
            "Parser@0/src/users/mod.rs",
            "Parser@0/src/auth/user.rs",
            "Parser@0/src/config.rs",
            "Parser@0/src/Parsers/json.rs",
            "Parser@0/src/handlers/http.rs",
        ]
    );
}

#[test]
fn explore_handler_matches_the_like_scan_results() {
    let db = seeded_db(3);
    assert_eq!(
        explore_snapshot(&db, "handler"),
        [
            "getUser@0/src/handlers/http.rs",
            "GetUser@0/src/handlers/http.rs",
            "get_user@0/src/handlers/http.rs",
            "GETTER@0/src/handlers/http.rs",
            "handler@0/src/handlers/http.rs",
            "HandleRequest@0/src/handlers/http.rs",
            "getUser1@1/src/handlers/http.rs",
            "GetUser1@1/src/handlers/http.rs",
            "get_user1@1/src/handlers/http.rs",
            "GETTER1@1/src/handlers/http.rs",
            "handler1@1/src/handlers/http.rs",
            "HandleRequest1@1/src/handlers/http.rs",
            "getUser2@2/src/handlers/http.rs",
            "GetUser2@2/src/handlers/http.rs",
            "get_user2@2/src/handlers/http.rs",
            "GETTER2@2/src/handlers/http.rs",
            "handler2@2/src/handlers/http.rs",
            "HandleRequest2@2/src/handlers/http.rs",
        ]
    );
}

fn symbols_table_scans(db: &GraphDb, sql: &str, params: impl rusqlite::Params) -> Vec<String> {
    let mut stmt = db
        .conn()
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let plan: Vec<String> = stmt
        .query_map(params, |row| row.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|line| line.starts_with("SEARCH s USING INDEX")),
        "{plan:#?}"
    );
    // `s` is the symbols alias inside the visible_symbols view; outer queries alias it otherwise.
    plan.into_iter()
        .filter(|line| line == "SCAN s" || line.starts_with("SCAN s "))
        .collect()
}

#[test]
fn prefix_tiers_seek_the_name_index() {
    let db = seeded_db(160);
    let [c1, c2, c3, c4] = prefix_casings("get");
    let sql = format!(
        "SELECT vs.id, f.path FROM visible_symbols vs JOIN visible_files f ON vs.file_id = f.id
         WHERE {} AND (?5 IS NULL OR vs.repo = ?5)
         ORDER BY length(vs.name), vs.id LIMIT 50",
        NAME_PREFIX_MATCH.replace("s.name", "vs.name")
    );
    let scans = symbols_table_scans(&db, &sql, rusqlite::params![c1, c2, c3, c4, None::<String>]);
    assert!(scans.is_empty(), "{scans:?}");
}

#[test]
fn path_tier_reaches_symbols_through_matched_files() {
    let db = seeded_db(160);
    let scans = symbols_table_scans(
        &db,
        PATH_TIER_SQL,
        rusqlite::params!["user", None::<String>],
    );
    assert!(scans.is_empty(), "{scans:?}");
}

#[test]
fn prefix_casings_dedupe_and_pad_with_the_typed_term() {
    assert_eq!(
        prefix_casings("getUser"),
        ["getUser", "getuser", "GetUser", "GETUSER"]
    );
    assert_eq!(prefix_casings("abc"), ["abc", "Abc", "ABC", "abc"]);
}

#[test]
fn get_impact_with_a_depth_beyond_i64_does_not_panic() {
    let db = GraphDb::open_in_memory().unwrap();
    db.insert_repo("app", "/app", "main", None).unwrap();
    let file_id = db.upsert_file("app", "src/lib.rs", "h", 1, 1).unwrap();
    let ids = db
        .insert_symbols(&[Symbol {
            id: None,
            file_id: Some(file_id),
            repo: "app".to_owned(),
            name: "root_fn".to_owned(),
            kind: SymbolKind::Fn,
            scope: None,
            signature: None,
            docstring: None,
            span: Span::new(1, 1, 2, 1),
            is_exported: true,
            complexity: None,
        }])
        .unwrap();
    let root_id = ids[0];

    let impact = crate::action::graph::query::get_impact(&db, root_id, usize::MAX)
        .expect("get_impact must not panic on an out-of-range depth");
    assert_eq!(impact.total_affected, 0);
}

#[test]
fn explore_find_surfaces_a_row_decode_error_instead_of_dropping_it() {
    let db = seeded_db(1);
    db.conn()
        .execute(
            "UPDATE symbols SET start_line = 'not-a-number' WHERE name = 'getUser'",
            [],
        )
        .unwrap();

    let err = explore_find(&db, "getUser", None, usize::MAX)
        .expect_err("a non-numeric start_line must surface as an error, not be silently dropped");
    assert!(matches!(
        err,
        crate::action::graph::query::types::QueryError::Sqlite(_)
    ));
}
