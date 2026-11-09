#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

fn declare_api_repo(root: &Utf8PathBuf) {
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
}

fn write_light_plan(root: &Utf8PathBuf, plan: &str) {
    fs::remove_path(&plan_file(root, "requirements.md")).unwrap();
    fs::remove_path(&plan_file(root, "analysis.md")).unwrap();
    fs::write_text(&plan_file(root, "plan.md"), plan).unwrap();
}

#[test]
fn approve_plan_refuses_repos_absent_from_the_manifest() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    declare_api_repo(&root);
    write_light_plan(&root, "---\nrepos: [api, billing]\n---\n# Plan\n");

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.unknown_repo");
    assert!(failure.what.contains("billing"), "{}", failure.what);
    assert!(!failure.what.contains("api"), "{}", failure.what);
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    assert_ne!(
        ApprovalState::read(&layout, &feature)
            .unwrap()
            .and_then(|state| state.state(Gate::Plan)),
        Some(GateState::Approved)
    );
}

#[test]
fn approve_plan_accepts_repos_declared_in_the_manifest() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    declare_api_repo(&root);
    write_light_plan(&root, "---\nrepos: [api]\n---\n# Plan\n");

    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(
        report.value.approvals.state(Gate::Plan),
        Some(GateState::Approved)
    );
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    assert_eq!(
        crate::action::plan::plan_declared_repos(&layout, &feature).unwrap(),
        vec![RepoName::new("api").unwrap()]
    );
}

fn api_worktree_with_web_package(root: &Utf8PathBuf) {
    let layout = Layout::at(root.clone());
    let web = layout
        .repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("main").unwrap(),
        )
        .join("packages/web");
    fs::ensure_dir(&web).unwrap();
    fs::write_text(&web.join("package.json"), r#"{"scripts":{"build":"tsc"}}"#).unwrap();
}

#[test]
fn approve_plan_refuses_commands_that_cannot_run_in_a_declared_repo() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    declare_api_repo(&root);
    api_worktree_with_web_package(&root);
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```bash\ncd apps/mobile\n```\n\n```bash\ncd packages/web\npnpm run deploy\npnpm run build\n```\n",
    );
    let tasks = plan_file(&root, "tasks");
    fs::ensure_dir(&tasks).unwrap();
    fs::write_text(
        &tasks.join("01-web.md"),
        "# Task\n```sh\ncd packages/web\nnpm run lint\n```\n",
    )
    .unwrap();

    let failure = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "plan.invalid_commands");
    let actual = failure.actual.unwrap();
    assert!(actual.contains("plan.md:7: `cd apps/mobile`"), "{actual}");
    assert!(actual.contains("plan.md:12: `pnpm run deploy`"), "{actual}");
    assert!(actual.contains("01-web.md:4: `npm run lint`"), "{actual}");
    assert!(!actual.contains("plan.md:11:"), "{actual}");
    assert!(!actual.contains("plan.md:13:"), "{actual}");
}

#[test]
fn approve_plan_accepts_commands_that_resolve_in_a_declared_repo() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    declare_api_repo(&root);
    api_worktree_with_web_package(&root);
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```bash\ncd packages/web\npnpm run build\n```\n\n```sh\nnpm run anything\n```\n",
    );

    let report = approve(
        &ctx,
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(
        report.value.approvals.state(Gate::Plan),
        Some(GateState::Approved)
    );
}

fn declare_api_and_web_repos(root: &Utf8PathBuf) {
    let layout = Layout::at(root.clone());
    let repo = |name: &str| {
        Repo::new(
            RepoName::new(name).unwrap(),
            format!("https://example.invalid/{name}.git"),
            BranchName::new("main").unwrap(),
        )
    };
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![repo("api"), repo("web")],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
}

fn approve_plan(root: &Utf8PathBuf) -> Outcome<ApproveOutcome> {
    approve(
        &Ctx::new(root.clone()),
        ApproveInput {
            feature: "checkout".to_owned(),
            gate: "plan".to_owned(),
        },
    )
}

#[test]
fn approve_plan_still_checks_scripts_when_another_repo_has_an_unparseable_package_json() {
    let (_guard, root) = seeded_hall();
    declare_api_and_web_repos(&root);
    let layout = Layout::at(root.clone());
    let main = BranchName::new("main").unwrap();
    let api = layout.repo_worktree(&RepoName::new("api").unwrap(), &main);
    let web = layout.repo_worktree(&RepoName::new("web").unwrap(), &main);
    fs::ensure_dir(&api).unwrap();
    fs::ensure_dir(&web).unwrap();
    fs::write_text(&api.join("package.json"), "{ not json").unwrap();
    fs::write_text(&web.join("package.json"), r#"{"scripts":{"build":"tsc"}}"#).unwrap();
    write_light_plan(
        &root,
        "---\nrepos: [api, web]\n---\n# Plan\n\n```sh\nnpm run deploy\n```\n",
    );

    let failure = approve_plan(&root).unwrap_err();

    assert_eq!(failure.code, "plan.invalid_commands");
    assert!(
        failure.actual.unwrap().contains("`npm run deploy`"),
        "the parsed package.json must still be checked"
    );
}

#[test]
fn approve_plan_accepts_a_cd_into_a_directory_the_plan_creates() {
    let (_guard, root) = seeded_hall();
    declare_api_repo(&root);
    api_worktree_with_web_package(&root);
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```sh\nmkdir -p packages/mobile\ncd packages/mobile\ncd src\nnpm run anything\n```\n",
    );

    let report = approve_plan(&root).unwrap();

    assert_eq!(
        report.value.approvals.state(Gate::Plan),
        Some(GateState::Approved)
    );
}

#[test]
fn approve_plan_refuses_a_cd_into_a_single_component_directory_nobody_creates() {
    let (_guard, root) = seeded_hall();
    declare_api_repo(&root);
    api_worktree_with_web_package(&root);
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```sh\ncd nonexistent\n```\n",
    );

    let failure = approve_plan(&root).unwrap_err();

    assert_eq!(failure.code, "plan.invalid_commands");
    assert!(
        failure.actual.unwrap().contains("`cd nonexistent`"),
        "a cd nothing creates must be refused"
    );
}

#[test]
fn a_directory_the_plan_creates_only_silences_its_own_block() {
    let (_guard, root) = seeded_hall();
    declare_api_repo(&root);
    api_worktree_with_web_package(&root);
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```sh\nmkdir -p packages/mobile\ncd packages/mobile\n```\n\n```sh\ncd packages/mobile\n```\n",
    );

    let failure = approve_plan(&root).unwrap_err();

    assert_eq!(failure.code, "plan.invalid_commands");
    let actual = failure.actual.unwrap();
    assert!(actual.contains("plan.md:12:"), "{actual}");
    assert!(!actual.contains("plan.md:8:"), "{actual}");
}

/// A feature created with `--base develop` plans against `develop`: a `cd`
/// into a directory that exists only on `develop` is runnable, even though
/// the default (`main`) worktree lacks it.
#[test]
fn approve_plan_resolves_commands_against_the_features_base_branch() {
    let (_guard, root) = seeded_hall();
    declare_api_repo(&root);
    let layout = Layout::at(root.clone());
    let checkout = FeatureName::new("checkout").unwrap();
    let mut record =
        crate::domain::feature::Feature::read_or_not_found(&layout, &checkout).unwrap();
    record.base = Some(BranchName::new("develop").unwrap());
    record.write(&layout).unwrap();
    let api = RepoName::new("api").unwrap();
    let main = layout.repo_worktree(&api, &BranchName::new("main").unwrap());
    let develop = layout.repo_worktree(&api, &BranchName::new("develop").unwrap());
    fs::ensure_dir(&main).unwrap();
    fs::ensure_dir(&develop.join("only-on-develop")).unwrap();
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```sh\ncd only-on-develop\n```\n",
    );

    let report = approve_plan(&root).unwrap();

    assert_eq!(
        report.value.approvals.state(Gate::Plan),
        Some(GateState::Approved)
    );
}

/// With the base worktree absent, the view falls back to the default
/// branch, and so does the command check.
#[test]
fn approve_plan_checks_the_default_worktree_when_the_base_worktree_is_absent() {
    let (_guard, root) = seeded_hall();
    declare_api_repo(&root);
    let layout = Layout::at(root.clone());
    let checkout = FeatureName::new("checkout").unwrap();
    let mut record =
        crate::domain::feature::Feature::read_or_not_found(&layout, &checkout).unwrap();
    record.base = Some(BranchName::new("develop").unwrap());
    record.write(&layout).unwrap();
    let main = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    fs::ensure_dir(&main).unwrap();
    write_light_plan(
        &root,
        "---\nrepos: [api]\n---\n# Plan\n\n```sh\ncd only-on-develop\n```\n",
    );

    let failure = approve_plan(&root).unwrap_err();

    assert_eq!(failure.code, "plan.invalid_commands");
    assert!(
        failure.actual.unwrap().contains("`cd only-on-develop`"),
        "the default worktree lacks the directory"
    );
}
