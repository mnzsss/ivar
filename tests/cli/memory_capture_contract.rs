#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use ivar::domain::capability::{Support, provider_support};
use ivar::domain::provider::Provider;

use crate::common::{hall_root, ivar};

const EVENTS: [&str; 4] = ["started", "prompted", "tool", "ended"];

fn declared(provider: Provider) -> BTreeSet<&'static str> {
    let support = provider_support(provider);
    [
        (support.session_started, "session.started"),
        (support.session_prompted, "session.prompted"),
        (support.session_tool, "session.tool"),
        (support.session_ended, "session.ended"),
    ]
    .into_iter()
    .filter(|(support, _)| *support == Support::Supported)
    .map(|(_, name)| name)
    .collect()
}

fn recorded(provider: Provider, name: &str) -> Option<String> {
    let path = format!(
        "{}/tests/cli/capture_payloads/{}/{name}.json",
        env!("CARGO_MANIFEST_DIR"),
        provider.id()
    );
    std::fs::read_to_string(path).ok()
}

fn init_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    (guard, root)
}

fn capture(root: &Utf8Path, provider: Provider, payload: &str) -> Duration {
    let started = Instant::now();
    ivar()
        .current_dir(root)
        .env("IVAR_HALL", root)
        .args(["memory", "capture", "--provider", provider.id()])
        .write_stdin(payload.to_owned())
        .assert()
        .success()
        .stdout("");
    started.elapsed()
}

fn status_events(root: &Utf8Path) -> BTreeSet<String> {
    let out = ivar()
        .current_dir(root)
        .args(["--json", "memory", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    value["observations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["source_event"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn each_provider_leaves_exactly_the_observations_its_matrix_declares_supported() {
    for provider in Provider::SELECTABLE {
        let (_guard, root) = init_hall();
        for file in EVENTS {
            if let Some(payload) = recorded(provider, file) {
                capture(&root, provider, &payload);
            }
        }
        let want: BTreeSet<String> = declared(provider).into_iter().map(str::to_owned).collect();
        assert_eq!(
            status_events(&root),
            want,
            "{provider}: fixtures must cover exactly the Supported events"
        );
    }
}

#[test]
fn the_hook_exits_zero_fast_with_a_locked_corrupt_or_missing_database() {
    let payload = recorded(Provider::ClaudeCode, "tool").unwrap();
    let ceiling = Duration::from_millis(500);
    let (_guard, root) = init_hall();
    let db = root.join(".ivar/memory.db");

    let holder = rusqlite::Connection::open(&db).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    assert!(
        capture(&root, Provider::ClaudeCode, &payload) < ceiling,
        "locked"
    );
    drop(holder);

    std::fs::write(&db, b"not a database").unwrap();
    assert!(
        capture(&root, Provider::ClaudeCode, &payload) < ceiling,
        "corrupt"
    );

    std::fs::remove_file(&db).unwrap();
    assert!(
        capture(&root, Provider::ClaudeCode, &payload) < ceiling,
        "missing"
    );
    assert!(!db.exists(), "the hook must not recreate the database");
}

#[test]
#[ignore = "set IVAR_CAPTURE_BENCH=1 and run with --profile e2e on a quiet machine"]
fn the_hook_p95_stays_inside_the_fifty_millisecond_budget() {
    if std::env::var_os("IVAR_CAPTURE_BENCH").is_none() {
        return;
    }
    let payload = recorded(Provider::ClaudeCode, "tool").unwrap();
    let (_guard, root) = init_hall();
    let mut samples: Vec<Duration> = (0..40)
        .map(|_| capture(&root, Provider::ClaudeCode, &payload))
        .collect();
    samples.sort();
    assert!(
        samples[37] < Duration::from_millis(50),
        "p95 {:?}",
        samples[37]
    );
}
