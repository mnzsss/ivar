#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::mcp::McpServerDef;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::harness::{commands, config};
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};
use camino::Utf8Path;

/// A hall with `repos` already declared in its `ivar.json`, plus the
/// origins those repos point at. Returns the hall root and the scratch dir
/// guard.
fn hall_with(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
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

    if !repos.is_empty() {
        let origins = root.parent().unwrap().join("origins");
        let declared: Vec<Repo> = repos
            .iter()
            .map(|(name, branch)| {
                let origin = seeded_repo(&origins.join(name), branch);
                Repo::new(
                    RepoName::new(*name).unwrap(),
                    origin.as_str(),
                    BranchName::new(*branch).unwrap(),
                )
            })
            .collect();

        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            declared,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
    }

    (guard, root)
}

fn entry<'a>(outcome: &'a SyncOutcome, surface: &str, label: &str) -> &'a Entry {
    outcome
        .entries
        .iter()
        .find(|e| e.surface == surface && e.label == label)
        .unwrap_or_else(|| panic!("no `{surface}` / `{label}` entry in {:?}", outcome.entries))
}

// -- setup scripts ---------------------------------------------------------

/// A setup script writes something a `git clone` never would — the whole
/// reason the hook exists.
fn write_setup_script(root: &Utf8Path, repo: &str, body: &str) {
    let script = Layout::at(root).setup_script(&RepoName::new(repo).unwrap());
    fs::ensure_dir(script.parent().unwrap()).unwrap();
    fs::write_text(&script, body).unwrap();
}

// -- official workflow commands -------------------------------------------

/// A hall whose `ivar.json` lists all providers.
fn hall_with_all_providers() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode, Provider::Omp],
            Provider::ClaudeCode,
        ),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    (guard, root)
}

/// The embedded source of the shipped command `id`.
fn embedded(id: &str) -> String {
    commands::catalog()
        .iter()
        .find(|command| command.id == id)
        .unwrap()
        .content
        .to_owned()
}

#[path = "sync/graph.rs"]
mod graph;
#[path = "sync/harness.rs"]
mod harness;
#[path = "sync/mcp.rs"]
mod mcp;
#[path = "sync/protection.rs"]
mod protection;
#[path = "sync/reporting.rs"]
mod reporting;
#[path = "sync/repos.rs"]
mod repos;
#[path = "sync/settings.rs"]
mod settings;
#[path = "sync/setup.rs"]
mod setup;
