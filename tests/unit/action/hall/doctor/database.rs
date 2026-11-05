#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::hall::{Diagnosis, DoctorOutcome, InitInput, doctor, init};
use crate::domain::mcp::McpServerDef;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;
use crate::store::memory::db::MemoryDb;
use crate::store::memory::schema::{APPLICATION_ID, MEMORY_SCHEMA_VERSION};

fn initialised_hall() -> (tempfile::TempDir, Utf8PathBuf, Layout) {
    let (guard, root) = crate::test_support::hall_root();
    init(
        &Ctx::new(root.clone()),
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let layout = Layout::at(root.clone());
    (guard, root, layout)
}

fn legacy_graph_at(layout: &Layout) {
    let db = GraphDb::open(layout.memory_db().as_std_path()).unwrap();
    db.insert_repo("legacy", "/x/legacy", "main", None).unwrap();
}

fn finding<'a>(report: &'a DoctorOutcome, code: &str) -> &'a Diagnosis {
    report
        .findings
        .iter()
        .find(|finding| finding.code == code)
        .unwrap_or_else(|| panic!("no `{code}` finding in {:?}", report.findings))
}

fn database_codes(report: &DoctorOutcome) -> Vec<&'static str> {
    report
        .findings
        .iter()
        .map(|finding| finding.code)
        .filter(|code| code.starts_with("memory.") || code.ends_with(".db_missing"))
        .collect()
}

#[test]
fn doctor_moves_a_legacy_graph_and_reports_nothing_about_the_databases() {
    let (_guard, root, layout) = initialised_hall();
    legacy_graph_at(&layout);

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(
        database_codes(&report.value).is_empty(),
        "{:?}",
        report.value.findings
    );
    let graph = GraphDb::open_read_only(layout.graph_db().as_std_path()).unwrap();
    assert!(graph.get_repo("legacy").unwrap().is_some());
    let memory = MemoryDb::inspect(layout.memory_db().as_std_path()).unwrap();
    assert_eq!(memory.application_id, APPLICATION_ID);
}

#[test]
fn doctor_names_a_legacy_graph_another_process_holds_open() {
    let (_guard, root, layout) = initialised_hall();
    legacy_graph_at(&layout);
    let holder = rusqlite::Connection::open(layout.memory_db().as_std_path()).unwrap();
    let _: i64 = holder
        .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
        .unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let busy = finding(&report.value, "memory.split_busy");
    assert!(busy.fix.contains("older `ivar`"), "{}", busy.fix);
    assert!(!layout.graph_db().as_std_path().exists());
    drop(holder);
}

#[test]
fn doctor_names_a_missing_graph_database_when_the_graph_mcp_is_registered() {
    let (_guard, root, layout) = initialised_hall();
    drop(MemoryDb::open(layout.memory_db().as_std_path()).unwrap());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let graph_server = McpServerDef::new("graph", "local")
        .command("ivar")
        .args(vec!["graph".to_owned(), "mcp".to_owned()]);
    Manifest::write(
        &layout,
        &manifest.with_mcp_servers(vec![graph_server]).unwrap(),
    )
    .unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let missing = finding(&report.value, "graph.db_missing");
    assert!(missing.fix.contains("ivar graph index"), "{}", missing.fix);
}

#[test]
fn doctor_says_nothing_about_a_graph_database_a_hall_never_built() {
    let (_guard, root, _layout) = initialised_hall();

    let report = doctor(&Ctx::new(root)).unwrap();

    assert!(
        database_codes(&report.value).is_empty(),
        "{:?}",
        report.value.findings
    );
}

#[test]
fn doctor_names_a_missing_memory_database_next_to_a_graph() {
    let (_guard, root, layout) = initialised_hall();
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());

    let report = doctor(&Ctx::new(root)).unwrap();

    let missing = finding(&report.value, "memory.db_missing");
    assert!(
        missing.fix.contains("ivar memory status"),
        "{}",
        missing.fix
    );
    assert!(
        !layout.memory_db().as_std_path().exists(),
        "doctor must not create it"
    );
}

#[test]
fn doctor_names_a_corrupt_memory_database() {
    let (_guard, root, layout) = initialised_hall();
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    std::fs::write(
        layout.memory_db(),
        b"not a sqlite database, just junk bytes",
    )
    .unwrap();

    let report = doctor(&Ctx::new(root)).unwrap();

    let corrupt = finding(&report.value, "memory.db_corrupt");
    assert!(corrupt.fix.contains("ivar memory clean"), "{}", corrupt.fix);
}

#[test]
fn doctor_names_a_memory_schema_newer_than_this_build() {
    let (_guard, root, layout) = initialised_hall();
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    let memory = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    memory
        .conn()
        .execute_batch(&format!(
            "UPDATE memory_schema SET version = {};",
            MEMORY_SCHEMA_VERSION + 1
        ))
        .unwrap();
    drop(memory);

    let report = doctor(&Ctx::new(root)).unwrap();

    let newer = finding(&report.value, "memory.schema_newer");
    assert!(newer.fix.contains("ivar upgrade"), "{}", newer.fix);
}

#[test]
fn doctor_names_a_memory_database_an_older_ivar_polluted_and_leaves_it_alone() {
    let (_guard, root, layout) = initialised_hall();
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    drop(MemoryDb::open(layout.memory_db().as_std_path()).unwrap());
    drop(GraphDb::open(layout.memory_db().as_std_path()).unwrap());

    let report = doctor(&Ctx::new(root)).unwrap();

    let polluted = finding(&report.value, "memory.db_polluted");
    assert!(
        polluted.fix.contains("ivar memory status"),
        "{}",
        polluted.fix
    );
    assert!(
        MemoryDb::inspect(layout.memory_db().as_std_path())
            .unwrap()
            .polluted
    );
}

#[test]
fn doctor_names_a_newer_memory_schema_with_extra_tables_as_newer_not_polluted() {
    let (_guard, root, layout) = initialised_hall();
    drop(GraphDb::open(layout.graph_db().as_std_path()).unwrap());
    let memory = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    memory
        .conn()
        .execute_batch(&format!(
            "UPDATE memory_schema SET version = {}; CREATE TABLE v2_pages (id TEXT);",
            MEMORY_SCHEMA_VERSION + 1
        ))
        .unwrap();
    drop(memory);

    let report = doctor(&Ctx::new(root)).unwrap();

    assert_eq!(database_codes(&report.value), vec!["memory.schema_newer"]);
}
