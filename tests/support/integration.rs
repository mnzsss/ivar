//! The integration-test adapter: linked from each top-level integration test
//! as `common`, sharing the helper implementation with the library's unit
//! adapter ([`shared`](shared)) while owning the helpers that only
//! integration tests need.
//!
//! `src/test_support.rs` is `#[cfg(test)]` inside the library, so it is not
//! part of the compiled crate and integration tests under `tests/` genuinely
//! cannot see it. That boundary is real and not worth breaking — but it does
//! not mean every test binary needs its own copy of the same four helpers,
//! which is what `tests/init.rs` and `tests/sync.rs` had before the shared
//! module existed.

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

pub(crate) use fake_gh::FakeGh;

use assert_cmd::Command;
use camino::Utf8Path;

/// The compiled `ivar` binary, ready to be given arguments.
pub(crate) fn ivar() -> Command {
    Command::cargo_bin("ivar").expect("binary builds")
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
