//! The unit-test adapter: linked from `src/lib.rs` as `crate::test_support`,
//! so library unit tests keep `use crate::test_support::…` unchanged.
//!
//! `#[cfg(test)]` items are not part of the compiled library, which is why
//! integration tests under `tests/` cannot see this module — they link
//! [`integration`](integration) instead. Both adapters pull their
//! implementation from [`shared`](shared), so the two boundaries share one
//! helper set rather than two drifting copies. It also links `fake_gh.rs`,
//! the same fake `gh` the integration adapter puts on `PATH`, and points this
//! thread's `gh` at it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

use crate::action::Ctx;
use crate::action::hall::{self, InitInput};

#[path = "shared.rs"]
mod shared;

pub(crate) use shared::{
    canonical_temp_dir, empty_repo, git, hall_root, seeded_repo, utf8_temp_dir,
};

#[path = "fake_gh.rs"]
mod fake_gh;

pub(crate) use fake_gh::FakeGh;

/// `fake`'s `gh` script with its state files in the environment — what a
/// test extends with a one-off switch such as `GH_FAKE_READY_FAIL`.
pub(crate) fn fake_gh_command(fake: &FakeGh) -> crate::infra::proc::Command {
    crate::infra::proc::Command::new(fake.dir.join("gh").as_str())
        .env("GH_FAKE_STATE", fake.state.as_str())
        .env("GH_FAKE_LOG", fake.log.as_str())
        .env("GH_FAKE_CHECKS", fake.checks.as_str())
}

/// Every `gh` this thread spawns runs `fake` until the guard drops — the
/// in-process counterpart of putting `fake.dir` first on a child's `PATH`.
pub(crate) fn fake_gh_on_this_thread(fake: &FakeGh) -> crate::infra::proc::Redirect {
    crate::infra::proc::redirect_on_this_thread("gh", fake_gh_command(fake))
}

#[path = "fake_t3.rs"]
mod fake_t3;

pub(crate) use fake_t3::FakeT3;

pub(crate) fn fake_t3_command(fake: &FakeT3) -> crate::infra::proc::Command {
    crate::infra::proc::Command::new("/bin/sh")
        .arg(fake.dir.join("t3").as_str())
        .env("T3_FAKE_PROJECTS", fake.projects.as_str())
        .env("T3_FAKE_LOG", fake.log.as_str())
}

pub(crate) fn redirect_t3(to: crate::infra::proc::Command) -> crate::infra::proc::Redirect {
    crate::infra::proc::redirect_on_this_thread(&crate::providers::t3::host::program(), to)
}

pub(crate) fn fake_t3_on_this_thread(fake: &FakeT3) -> crate::infra::proc::Redirect {
    redirect_t3(fake_t3_command(fake))
}

#[path = "fake_docker.rs"]
mod fake_docker;

pub(crate) use fake_docker::FakeDocker;

pub(crate) fn fake_docker_command(fake: &FakeDocker) -> crate::infra::proc::Command {
    crate::infra::proc::Command::new("/bin/sh")
        .arg(fake.dir.join("docker").as_str())
        .env("DOCKER_FAKE_LOG", fake.log.as_str())
}

pub(crate) fn fake_docker_on_this_thread(fake: &FakeDocker) -> crate::infra::proc::Redirect {
    crate::infra::proc::redirect_on_this_thread("docker", fake_docker_command(fake))
}

#[path = "fake_systemctl.rs"]
mod fake_systemctl;

pub(crate) use fake_systemctl::FakeSystemctl;

pub(crate) fn fake_systemctl_command(fake: &FakeSystemctl) -> crate::infra::proc::Command {
    crate::infra::proc::Command::new("/bin/sh")
        .arg(fake.dir.join("systemctl").as_str())
        .env("SYSTEMCTL_FAKE_LOG", fake.log.as_str())
}

pub(crate) fn fake_systemctl_on_this_thread(fake: &FakeSystemctl) -> crate::infra::proc::Redirect {
    crate::infra::proc::redirect_on_this_thread("systemctl", fake_systemctl_command(fake))
}

/// Rewrites the hall manifest to list `claude-code` and `t3`, with `default` as the default.
pub(crate) fn enable_t3_host(root: &Utf8PathBuf, default: crate::domain::provider::Provider) {
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers};
    let layout = crate::store::layout::Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let updated = manifest
        .with_providers(Providers::new(
            vec![Provider::ClaudeCode, Provider::T3],
            default,
        ))
        .unwrap();
    Manifest::write(&layout, &updated).unwrap();
}

/// A canonicalised hall root with a freshly initialised hall named `acme`.
///
/// Shared by every action-layer unit-test module that needs a real hall to
/// operate against rather than a bare directory: initialising once here means
/// the 14 call sites that used to carry their own byte-identical copy stay in
/// sync automatically when `hall::init`'s input shape changes.
pub(crate) fn seeded_hall() -> (TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    (guard, root)
}

/// Creates the hall's graph database and a live discovery session, returning
/// the session's view dir, so app-layer tests can drive graph commands
/// without importing `store`.
pub(crate) fn graph_session_view(
    root: &Utf8PathBuf,
    session_id: &crate::domain::name::SessionId,
) -> Utf8PathBuf {
    let layout = crate::store::layout::Layout::at(root.clone());
    crate::store::graph::db::GraphDb::open(layout.ivar_dir().join("memory.db").as_std_path())
        .unwrap();
    let view_dir = layout.discovery_session(session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::domain::session::SessionState::new(
        crate::domain::provider::Provider::ClaudeCode,
        "2026-08-29T00:00:00Z",
    )
    .write(&view_dir)
    .unwrap();
    view_dir
}

/// The query text of the latest graph call recorded for `session`.
pub(crate) fn last_graph_query(root: &Utf8PathBuf, session: &str) -> Option<Option<String>> {
    let db_path = crate::store::layout::Layout::at(root.clone())
        .ivar_dir()
        .join("memory.db");
    crate::store::graph::db::GraphDb::open(db_path.as_std_path())
        .unwrap()
        .last_graph_call(session)
        .unwrap()
        .map(|call| call.query)
}

/// Seeds a hall root's git hooks and config and each provider's hook config,
/// next to unprotected siblings (`.git/objects`, `.git/index`,
/// `.claude/skills`, `docs`).
pub(crate) fn seed_protected_hall_paths(root: &Utf8Path) {
    for dir in [
        ".git/hooks",
        ".git/objects",
        ".claude/skills",
        ".opencode/plugins",
        ".omp/hooks/pre",
        ".omp/extensions",
        "docs",
    ] {
        crate::infra::fs::ensure_dir(&root.join(dir)).unwrap();
    }
    for file in [
        ".git/config",
        ".git/index",
        ".claude/settings.json",
        ".opencode/plugins/ivar.js",
        ".omp/hooks/pre/ivar.js",
        ".omp/extensions/ivar.js",
    ] {
        crate::infra::fs::write_text(&root.join(file), "").unwrap();
    }
}

/// Runs `operation` while another thread holds the graph index lock for
/// ~300 ms, and asserts it only returned after that holder let go.
pub(crate) fn assert_waits_for_index_lock(
    layout: &crate::store::layout::Layout,
    operation: impl FnOnce(),
) {
    use std::sync::atomic::{AtomicBool, Ordering};

    let released = AtomicBool::new(false);
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let guard = crate::action::graph::lock_index(layout).unwrap();
            locked_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            released.store(true, Ordering::SeqCst);
            drop(guard);
        });
        locked_rx.recv().unwrap();
        operation();
        assert!(
            released.load(Ordering::SeqCst),
            "the operation finished while the index lock was still held"
        );
    });
}

pub(crate) fn process_is_gone(pid: &str) -> bool {
    (0..20).any(|_| {
        let gone = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_or(true, |stat| stat.split_whitespace().nth(2) == Some("Z"));
        if !gone {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        gone
    })
}
