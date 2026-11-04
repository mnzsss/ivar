#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::os::unix::fs::PermissionsExt as _;

use super::*;
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::action::feature::promote::{self, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

#[derive(Debug)]
struct RecordingConfirm {
    answer: bool,
    captured_question: std::sync::Mutex<Option<String>>,
    captured_caveat: std::sync::Mutex<Option<String>>,
}

impl RecordingConfirm {
    fn new(answer: bool) -> Self {
        Self {
            answer,
            captured_question: std::sync::Mutex::new(None),
            captured_caveat: std::sync::Mutex::new(None),
        }
    }
}

impl crate::action::confirm::Confirm for RecordingConfirm {
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure> {
        *self.captured_question.lock().unwrap() = Some(question.to_owned());
        *self.captured_caveat.lock().unwrap() = caveat.map(str::to_owned);
        Ok(self.answer)
    }

    fn select_many(
        &self,
        _prompt: &str,
        _options: &[crate::action::confirm::SelectOption],
    ) -> Result<Vec<usize>, Failure> {
        Ok(Vec::new())
    }

    fn select_one(
        &self,
        _prompt: &str,
        _options: &[crate::action::confirm::SelectOption],
    ) -> Result<Option<usize>, Failure> {
        Ok(None)
    }

    fn is_interactive(&self) -> bool {
        true
    }
}

/// A hall with one seeded repo declared, a feature created, and the repo
/// promoted (so a real worktree exists to tear down).
fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
    hall_with_promoted_feature_on(None)
}

/// The same hall, with the feature on an explicit branch. A branch holding a
/// `/` — `feat/checkout` — nests the worktree one directory deeper, which is
/// what the orphan-parent teardown has to cope with.
fn hall_with_promoted_feature_on(branch: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    create_action(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: branch.map(str::to_owned),
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    (guard, root)
}

fn delete_input(name: &str) -> DeleteInput {
    DeleteInput {
        name: name.to_owned(),
        force: false,
        descendants: Descendants::Ask,
    }
}

#[path = "delete/removal.rs"]
mod removal;
#[path = "delete/subtree.rs"]
mod subtree;
