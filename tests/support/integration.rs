//! The integration-test adapter: linked from each top-level integration test
//! as `common`, sharing the helper implementation with the library's unit
//! adapter ([`shared`](shared)) while owning the helpers that only
//! integration tests need.
//!
//! `src/test_support.rs` is `#[cfg(test)]` inside the library, so it is not
//! part of the compiled crate and integration tests under `tests/` genuinely
//! cannot see it. That boundary is real and not worth breaking — but it does
//! not mean every test binary needs its own copy of the same helpers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

#[path = "shared.rs"]
mod shared;

pub(crate) use shared::*;

#[path = "fake_gh.rs"]
mod fake_gh;

// Used by the delivery and cli binaries; personas and provider_live include
// this module without using it.
#[allow(unused_imports)]
pub(crate) use fake_gh::FakeGh;

use assert_cmd::Command;
use camino::Utf8Path;

/// The compiled `ivar` binary, ready to be given arguments.
pub(crate) fn ivar() -> Command {
    let mut command = Command::cargo_bin("ivar").expect("binary builds");
    command
        .env("IVAR_NO_UPDATE_CHECK", "1")
        .env_remove("IVAR_FEATURE");
    command
}

/// Rewrite a hall's `ivar.json` to declare `repos` as `(name, url, branch)`.
///
/// Written by hand rather than through a verb because `ivar repo add` is a later
/// slice — and because `ivar.json` being hand-editable is the contract, so a
/// test that hand-edits it is testing the real path.
pub(crate) fn declare_repos(root: &Utf8Path, repos: &[(&str, &Utf8Path, &str)]) {
    let entries = repos
        .iter()
        .map(|(name, url, branch)| {
            format!(r#"{{"default_branch":"{branch}","name":"{name}","url":"{url}"}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        root.join("ivar.json"),
        format!(
            r#"{{"name":"acme","providers":{{"available":["claude-code"],"default":"claude-code"}},"repos":[{entries}],"version":1}}"#
        ),
    )
    .expect("write ivar.json");
}

/// Appends a `memory.reviewed` for `feature` to the delivery spool, where
/// `ivar feature deliver` drains it from.
pub(crate) fn record_memory_review(root: &Utf8Path, feature: &str) {
    let dir = root.join(".ivar/spool/memory");
    std::fs::create_dir_all(&dir).unwrap();
    let line = serde_json::json!({
        "event_id": format!("fixture-review-{feature}"),
        "source_event": "memory.reviewed",
        "occurred_at": "2026-01-01T00:00:00Z",
        "session_id": "fixture-session",
        "native_session_id": "",
        "provider": "",
        "ivar_version": "",
        "feature": feature,
        "parent_feature": "",
        "repo": "",
        "origin": "",
        "importance": 5,
        "attempt_id": "",
        "body": "{\"heads\":{}}",
    });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("delivery.jsonl"))
        .unwrap();
    std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes()).unwrap();
}
