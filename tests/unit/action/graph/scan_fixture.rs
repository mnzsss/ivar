//! Seeded graph databases built once and copied for each test.
//!
//! Seeding 20k symbols takes seconds, copying the file takes milliseconds.
//! The template is a file in `<target>/<profile>/ivar-test-templates` rather
//! than a `OnceLock`, so that nextest, which runs every test in its own
//! process, still seeds it once. Its name carries the schema version, the
//! caller's name for the shape, and a digest of the seeding file's source, so
//! a migration or an edited seed builds a new template instead of copying a
//! stale one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::domain::graph::{Span, Symbol, SymbolKind};
use crate::store::graph::db::GraphDb;
use crate::store::graph::schema::SCHEMA_VERSION;

static SEEDING: Mutex<()> = Mutex::new(());

/// A private copy of the template `name`, seeded by `seed` when no process
/// has built it yet. `seed_source` is the text of the file that defines
/// `seed` (pass `include_str!` of it).
pub(super) fn seeded_copy(
    name: &str,
    seed_source: &str,
    seed: impl FnOnce(&GraphDb),
) -> (tempfile::TempDir, GraphDb) {
    let template = template_path(name, seed_source);
    {
        let _seeding = SEEDING.lock().unwrap_or_else(PoisonError::into_inner);
        if !template.is_file() {
            build_template(&template, seed);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("graph.db");
    std::fs::copy(&template, &copy).unwrap();
    let db = GraphDb::open(&copy).unwrap();
    (dir, db)
}

fn template_path(name: &str, seed_source: &str) -> PathBuf {
    let digest = crate::infra::hash::text(seed_source);
    let short = digest.get(..16).unwrap_or(&digest);
    templates_dir().join(format!("scan-fixture-v{SCHEMA_VERSION}-{name}-{short}.db"))
}

/// `<target>/<profile>/ivar-test-templates`: every test binary runs from
/// `<target>/<profile>/deps/`.
fn templates_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().and_then(Path::parent).unwrap();
    profile_dir.join("ivar-test-templates")
}

// Concurrent nextest processes may each build the template. Each writes its
// own partial file and renames it into place; the rename is atomic, so a
// reader copies either no template or a complete one, never a torn file.
fn build_template(template: &Path, seed: impl FnOnce(&GraphDb)) {
    std::fs::create_dir_all(template.parent().unwrap()).unwrap();
    let db = GraphDb::open_in_memory().unwrap();
    seed(&db);
    let partial = template.with_extension(format!("{}.partial", std::process::id()));
    let _ = std::fs::remove_file(&partial);
    db.conn()
        .execute("VACUUM INTO ?1", [partial.to_str().unwrap()])
        .unwrap();
    std::fs::rename(&partial, template).unwrap();
}

fn one_symbol(db: &GraphDb) {
    db.insert_repo("app", "/app", "main", None).unwrap();
    let file_id = db.upsert_file("app", "src/lib.rs", "h", 1, 1).unwrap();
    db.insert_symbols(&[Symbol {
        id: None,
        file_id: Some(file_id),
        repo: "app".to_owned(),
        name: "only".to_owned(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span::new(1, 1, 2, 1),
        is_exported: true,
        complexity: None,
    }])
    .unwrap();
}

fn symbol_count(db: &GraphDb) -> i64 {
    db.conn()
        .query_row("SELECT count(*) FROM symbols", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_copy_carries_the_seeded_rows_at_the_current_schema_version() {
    let (_dir, db) = seeded_copy("selftest-rows", "selftest rows v1", one_symbol);

    assert_eq!(symbol_count(&db), 1);
    let version: i64 = db
        .conn()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn writes_to_one_copy_never_reach_another() {
    let (_first_dir, first) =
        seeded_copy("selftest-isolation", "selftest isolation v1", one_symbol);
    first.conn().execute("DELETE FROM symbols", []).unwrap();

    let (_second_dir, second) =
        seeded_copy("selftest-isolation", "selftest isolation v1", one_symbol);

    assert_eq!(symbol_count(&first), 0);
    assert_eq!(symbol_count(&second), 1);
}

#[test]
fn the_template_is_seeded_at_most_once() {
    let seeds = AtomicUsize::new(0);
    let counting_seed = |db: &GraphDb| {
        seeds.fetch_add(1, Ordering::SeqCst);
        one_symbol(db);
    };

    let _first = seeded_copy("selftest-once", "selftest once v1", counting_seed);
    let after_first = seeds.load(Ordering::SeqCst);
    let _second = seeded_copy("selftest-once", "selftest once v1", counting_seed);

    assert!(after_first <= 1, "seeded {after_first} times");
    assert_eq!(seeds.load(Ordering::SeqCst), after_first);
}

#[test]
fn a_changed_seed_or_schema_gets_its_own_template() {
    let current = template_path("selftest-name", "seed a");

    assert_ne!(current, template_path("selftest-name", "seed b"));
    assert_ne!(current, template_path("selftest-other", "seed a"));
    assert!(
        current
            .to_str()
            .unwrap()
            .contains(&format!("scan-fixture-v{SCHEMA_VERSION}-selftest-name-")),
        "{current:?}"
    );
    assert_eq!(current.parent(), Some(templates_dir().as_path()));
}
