#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::{Utf8Path, Utf8PathBuf};
use rstest::rstest;

use super::*;
use crate::action::feature::create::{CreateInput, create as feature_create};
use crate::domain::feature::{RunBaseline, RunId, RunReceipt};
use crate::domain::name::{FeatureName, HallName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Status, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, MigrationPlan, Providers};
use crate::test_support::{
    FakeT3, enable_t3_host, fake_t3_command, fake_t3_on_this_thread, hall_root,
    hall_root as utf8_temp_dir,
};

fn fresh_input() -> InitInput {
    InitInput {
        path: Utf8PathBuf::from("."),
        name: None,
        provider: None,
        git: false,
        origin: None,
    }
}

// -----------------------------------------------------------------------
// `ivar migrate`
// -----------------------------------------------------------------------

/// A hall whose `ivar.json` has been rewritten to `version`, returning the
/// raw bytes now on disk so a test can prove they were left alone.
fn hall_at_version(root: &Utf8Path, version: u32) -> String {
    let path = root.join("ivar.json");
    let text = fs::read_text(&path).unwrap().unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("version".to_owned(), serde_json::Value::from(version));
    fs::write_text(
        &path,
        &format!("{}\n", serde_json::to_string(&value).unwrap()),
    )
    .unwrap();
    fs::read_text(&path).unwrap().unwrap()
}

fn human(outcome: &MigrateOutcome) -> String {
    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

// -- init --git / --origin -------------------------------------------------

fn git_out(cwd: &Utf8Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn init_with(path: &str, git: bool, origin: Option<&str>) -> InitInput {
    InitInput {
        path: Utf8PathBuf::from(path),
        name: Some("acme".to_owned()),
        provider: None,
        git,
        origin: origin.map(str::to_owned),
    }
}

const ORIGIN: &str = "git@example.com:acme/hall.git";

// -- status ---------------------------------------------------------------

fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();

    let origin = crate::test_support::seeded_repo(
        &root.parent().unwrap().join("origins").join("api"),
        "main",
    );
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![crate::store::manifest::Repo::new(
            crate::domain::name::RepoName::new("api").unwrap(),
            origin.as_str(),
            crate::domain::name::BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    (guard, root)
}

fn synced_hall_with_bare() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (guard, root) = hall_with_repo();
    crate::action::sync::sync(&Ctx::new(root.clone()), &Default::default()).unwrap();
    let bare = root.join(".ivar/repos/api/.bare");
    (guard, root, bare)
}

fn orphan_findings(report: &DoctorOutcome) -> Vec<&Diagnosis> {
    report
        .findings
        .iter()
        .filter(|finding| finding.code == "repo.worktree_orphaned")
        .collect()
}

// -- doctor: shipped workflow commands ------------------------------------

fn finding<'a>(report: &'a DoctorOutcome, code: &str) -> &'a Diagnosis {
    report
        .findings
        .iter()
        .find(|finding| finding.code == code)
        .unwrap_or_else(|| panic!("no `{code}` finding in {:?}", report.findings))
}

// -- doctor: codebase graph -----------------------------------------------

fn hall_with_indexed_graph() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    drop(
        crate::store::graph::db::GraphDb::open(root.join(".ivar/graph.db").as_std_path()).unwrap(),
    );
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    (guard, root)
}

use crate::domain::mcp::{CredentialState, McpOauth, McpServerDef};

#[path = "hall/cleanup.rs"]
mod cleanup;
#[path = "hall/doctor_graph.rs"]
mod doctor_graph;
#[path = "hall/doctor_hall.rs"]
mod doctor_hall;
#[path = "hall/doctor_instructions.rs"]
mod doctor_instructions;
#[path = "hall/doctor_worktrees.rs"]
mod doctor_worktrees;
#[path = "hall/init.rs"]
mod init;
#[path = "hall/migrate.rs"]
mod migrate;
#[path = "hall/status.rs"]
mod status;
