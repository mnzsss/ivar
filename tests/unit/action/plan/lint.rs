#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::plan_lint::{CREATE_EXISTS, MODIFY_MISSING, PLACEHOLDER};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::hall_root;

const GOOD: &str = "### Task 1: Thing\n\
**Files:**\n\
- Create: `src/new.rs`\n\
- Modify: `src/lib.rs:1-3`\n\
**Readers:**\n\
- `git grep -n thing`\n\
\x20 ```\n\
\x20 src/lib.rs:2:thing\n\
\x20 ```\n\
**Interfaces:**\n\
- Consumes: none\n\
- Produces: none\n\
- [ ] **Step 1: Write the failing test**\n";

fn hall() -> (tempfile::TempDir, Utf8PathBuf) {
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
    feature_create::create(
        &ctx,
        FeatureCreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            "https://example.invalid/api.git",
            BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let src = layout
        .repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("main").unwrap(),
        )
        .join("src");
    fs::ensure_dir(&src).unwrap();
    fs::write_text(&src.join("lib.rs"), "fn thing() {}\n").unwrap();
    (guard, root)
}

fn write_plan_and_packet(root: &Utf8PathBuf, packet: &str) {
    let layout = Layout::at(root.clone());
    let dir = layout.plan_dir(&FeatureName::new("checkout").unwrap());
    fs::write_text(&dir.join("plan.md"), "---\nrepos: [api]\n---\n# Plan\n").unwrap();
    fs::ensure_dir(&dir.join("tasks")).unwrap();
    fs::write_text(&dir.join("tasks/01-thing.md"), packet).unwrap();
}

fn lint_checkout(root: &Utf8PathBuf) -> Outcome<LintOutcome> {
    lint(
        &Ctx::new(root.clone()),
        LintInput {
            feature: "checkout".to_owned(),
        },
    )
}

#[test]
fn a_valid_packet_lints_clean_against_the_declared_repos() {
    let (_guard, root) = hall();
    write_plan_and_packet(&root, GOOD);

    let report = lint_checkout(&root).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert_eq!(report.value.packets, ["01-thing.md"]);
    assert_eq!(report.value.problems, 0);
}

#[test]
fn every_problem_is_its_own_warning_with_its_own_code() {
    let (_guard, root) = hall();
    let packet = GOOD
        .replace("Create: `src/new.rs`", "Create: `src/lib.rs`")
        .replace("Modify: `src/lib.rs:1-3`", "Modify: `src/gone.rs:1-3`")
        + "- [ ] **Step 3: Implement** TODO\n";
    write_plan_and_packet(&root, &packet);

    let report = lint_checkout(&root).unwrap();

    let codes: Vec<&str> = report.warnings.iter().map(|w| w.code).collect();
    assert_eq!(codes, [CREATE_EXISTS, MODIFY_MISSING, PLACEHOLDER]);
    assert_eq!(report.value.problems, 3);
    assert!(report.warnings[0].subject.starts_with("01-thing.md:"));
    assert!(report.warnings[0].what.starts_with(CREATE_EXISTS));
}

#[test]
fn a_path_may_carry_the_repo_name_as_a_prefix() {
    let (_guard, root) = hall();
    write_plan_and_packet(
        &root,
        &GOOD.replace("Modify: `src/lib.rs", "Modify: `api/src/lib.rs"),
    );

    assert!(lint_checkout(&root).unwrap().is_clean());
}

#[test]
fn a_feature_without_packets_is_blocked() {
    let (_guard, root) = hall();

    let failure = lint_checkout(&root).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.no_packets");
}

#[test]
fn an_unknown_feature_is_blocked() {
    let (_guard, root) = hall();

    let failure = lint(
        &Ctx::new(root),
        LintInput {
            feature: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "plan.feature_not_found");
}

#[test]
fn the_human_surface_counts_packets_and_problems() {
    let outcome = LintOutcome {
        root: Utf8PathBuf::from("/hall"),
        feature: FeatureName::new("checkout").unwrap(),
        packets: vec!["01-a.md".to_owned(), "02-b.md".to_owned()],
        problems: 2,
    };
    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Linted 2 task packet(s) of `checkout`: 2 problem(s)\n"
    );
}
