#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::action::feature::create::{CreateInput, create as feature_create};
use crate::domain::feature::{RunBaseline, RunId, RunReceipt};
use crate::domain::name::{FeatureName, HallName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Status, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, MigrationPlan, Providers};
use crate::test_support::{hall_root, hall_root as utf8_temp_dir};

fn fresh_input() -> InitInput {
    InitInput {
        path: Utf8PathBuf::from("."),
        name: None,
        provider: None,
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

#[test]
fn migrate_on_a_current_hall_reports_nothing_to_do_and_writes_nothing() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    let before = fs::read_text(&root.join("ivar.json")).unwrap().unwrap();

    let report = migrate(&ctx).unwrap();

    assert_eq!(report.value.plan, MigrationPlan::Current { version: 3 });
    assert!(!report.value.migrated);
    assert!(report.is_clean());
    assert!(human(&report.value).contains("Nothing to do"));
    assert_eq!(
        fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
        before,
        "a no-op migrate must not rewrite the file"
    );
}

#[test]
fn migrate_outside_a_hall_is_blocked() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let failure = migrate(&ctx).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn migrate_refuses_a_file_newer_than_this_build_without_touching_it() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    let on_disk = hall_at_version(&root, 99);

    let report = migrate(&ctx).unwrap();

    assert_eq!(
        report.value.plan,
        MigrationPlan::TooNew {
            found: 99,
            highest: 3
        }
    );
    assert!(!report.value.migrated);
    // The whole point of `plan` over `read`: a too-new hall gets described,
    // not refused into silence.
    assert!(human(&report.value).contains("understands up to 3"));
    // ...but describing it must not report success. A warning is what
    // makes `bin/ivar.rs` exit 1 instead of 0.
    assert!(!report.is_clean(), "a too-new hall must not exit clean");
    assert_eq!(report.warnings[0].code, "hall.manifest_too_new");
    assert_eq!(
        fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
        on_disk,
        "a too-new file must never be modified"
    );
}

#[test]
fn migrate_reports_an_unversioned_file_as_unreachable_rather_than_adopting_it() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    let on_disk = hall_at_version(&root, 0);

    let report = migrate(&ctx).unwrap();

    // `ivar.json`'s first public version is 1, so there is no v0 to migrate
    // from. Relabelling it as current would adopt a foreign file as ours —
    // the format contract forbids exactly that.
    assert_eq!(
        report.value.plan,
        MigrationPlan::Unreachable { from: 0, to: 3 }
    );
    assert!(!report.value.migrated);
    assert!(human(&report.value).contains("no migration to reach version 3"));
    assert!(
        !report.is_clean(),
        "an unreachable hall must not exit clean"
    );
    assert_eq!(report.warnings[0].code, "hall.manifest_unreachable");
    assert_eq!(
        fs::read_text(&root.join("ivar.json")).unwrap().unwrap(),
        on_disk
    );
}

#[test]
fn a_non_prompting_run_never_answers_yes() {
    // The safety property both `cleanup` and `migrate` rest on: a Ctx built
    // without an explicit confirmer (the test-suite case, and the `--json`,
    // `$CI`, and non-tty cases in the binary) never consents. A pipe is not
    // consent.
    let (_tmp, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);
    assert!(!ctx.confirm("Delete everything?", None).unwrap());
    assert!(!ctx.confirm("Rewrite it?", Some("careful")).unwrap());
}

#[test]
fn cleanup_removes_when_the_confirmation_seam_says_yes() {
    // The seam, not the terminal, decides: a test that installs a fixed yes
    // gets the deletion half of the prompt deterministically.
    let (_guard, root) = utf8_temp_dir();
    let mut ctx = Ctx::new(root.clone());
    ctx = ctx.with_confirm(crate::action::confirm::fixed(true));
    init(&ctx, fresh_input()).unwrap();

    // A stale repo directory — not in the manifest.
    fs::ensure_dir(&root.join(".ivar/repos/stale")).unwrap();

    let report = cleanup(&ctx).unwrap();

    assert_eq!(report.value.removed.len(), 1);
    assert!(report.value.removed[0].contains("stale"));
    assert!(!fs::exists(&root.join(".ivar/repos/stale")).unwrap());
}

#[test]
fn init_creates_the_expected_on_disk_shape() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());

    let report = init(&ctx, fresh_input()).unwrap();

    assert!(report.is_clean());
    assert!(fs::is_file(&root.join("ivar.json")).unwrap());
    assert!(fs::is_dir(&root.join(".ivar")).unwrap());
    assert!(fs::is_file(&root.join(".gitignore")).unwrap());
    assert_eq!(report.value.root, root);
}

#[test]
fn init_derives_the_name_from_the_directory_when_absent() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());

    let report = init(&ctx, fresh_input()).unwrap();

    let expected_name = root.file_name().unwrap();
    assert_eq!(report.value.name.as_str(), expected_name);
}

#[test]
fn init_defaults_the_provider_to_claude_code() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let report = init(&ctx, fresh_input()).unwrap();

    assert_eq!(report.value.provider, Provider::ClaudeCode);
}

#[test]
fn init_honours_an_explicit_name_and_provider() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let report = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(report.value.name.as_str(), "acme");
    assert_eq!(report.value.provider, Provider::OpenCode);
}

#[test]
fn init_rejects_a_second_init_in_the_same_directory() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    init(&ctx, fresh_input()).unwrap();
    let error = init(&ctx, fresh_input()).unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "hall.already_initialised");
    assert!(!error.fix_actions.is_empty());
}

#[test]
fn init_rejects_nesting_inside_an_existing_hall() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();

    let nested = root.join("nested");
    fs::ensure_dir(&nested).unwrap();
    let error = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("nested"),
            name: None,
            provider: None,
        },
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "hall.nested");
}

#[test]
fn init_rejects_an_invalid_explicit_name() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let error = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("../etc".to_owned()),
            provider: None,
        },
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "name.not_a_segment");
}

#[test]
fn init_rejects_an_invalid_derived_name_with_an_extra_fix_action() {
    let (_guard, root) = utf8_temp_dir();
    let hidden = root.join(".hidden");
    fs::ensure_dir(&hidden).unwrap();
    let ctx = Ctx::new(hidden);

    let error = init(&ctx, fresh_input()).unwrap_err();

    assert_eq!(error.code, "name.hidden");
    assert!(
        error
            .fix_actions
            .iter()
            .any(|fix| fix.code == "hall.pass_name")
    );
}

#[test]
fn init_rejects_an_invalid_provider_id() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let error = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: Some("claude".to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "provider.unknown_id");
}

#[test]
fn gitignore_uses_the_star_form_and_reincludes_committed_children() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());

    init(&ctx, fresh_input()).unwrap();

    let content = fs::read_text(&root.join(".gitignore")).unwrap().unwrap();
    assert_eq!(
        content,
        ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n\
             .claude/commands/ivar-*.md\n.opencode/commands/ivar-*.md\n\
             .claude/skills/\n.opencode/skills/\n"
    );
    assert!(!content.lines().any(|line| line == ".ivar/"));
}

#[test]
fn gitignore_preserves_existing_content_and_does_not_duplicate_on_rerun() {
    let (_guard, root) = utf8_temp_dir();
    fs::write_text(&root.join(".gitignore"), "node_modules/\n").unwrap();
    let ctx = Ctx::new(root.clone());

    init(&ctx, fresh_input()).unwrap();

    let content = fs::read_text(&root.join(".gitignore")).unwrap().unwrap();
    assert!(content.starts_with("node_modules/\n"));
    assert_eq!(content.matches(".ivar/*").count(), 1);
}

// -- shipped workflow commands --------------------------------------------

#[test]
fn init_materialises_commands_for_its_selected_provider() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());

    let report = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    for command in crate::harness::commands::catalog() {
        assert!(
            fs::is_file(&root.join(".opencode/commands").join(command.file_name())).unwrap(),
            "{} should be materialised for opencode",
            command.file_name()
        );
    }
    assert!(
        !fs::exists(&root.join(".claude")).unwrap(),
        "a Claude command directory must not exist for an OpenCode hall"
    );
}

#[test]
fn init_returns_warning_when_command_materialisation_fails() {
    let (_guard, root) = utf8_temp_dir();
    // Occupy OpenCode's command-directory parent with a regular file, so
    // `ensure_dir` cannot create `.opencode/commands` under it.
    fs::write_text(&root.join(".opencode"), "not a directory\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        },
    )
    .unwrap();

    assert!(
        !report.is_clean(),
        "a failed command write must not be clean"
    );
    assert_eq!(
        report.warnings[0].code,
        "provider.commands_not_materialised"
    );
    assert_eq!(report.warnings[0].subject, "opencode");
    assert!(report.warnings[0].what.contains("ivar sync"));
    // The manifest is still valid and still selects OpenCode.
    let on_disk = fs::read_text(&root.join("ivar.json")).unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
    assert_eq!(
        value["providers"]["available"],
        serde_json::json!(["opencode"])
    );
}

#[test]
fn write_human_names_the_hall_root_and_provider() {
    let outcome = InitOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: HallName::new("acme").unwrap(),
        provider: Provider::ClaudeCode,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Initialised hall `acme` at /hall (provider: claude-code)\n"
    );
}

// -- status ---------------------------------------------------------------

fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
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
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    (guard, root)
}

#[test]
fn status_reports_a_fresh_hall_as_operational() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();

    let report = status(&ctx).unwrap();

    assert_eq!(report.value.health, "operational");
    assert!(report.value.repos.is_empty());
}

#[test]
fn status_reports_a_synced_hall_with_repos_as_operational() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let report = status(&ctx).unwrap();

    assert_eq!(report.value.health, "operational");
    assert_eq!(report.value.repos.len(), 1);
    assert!(report.value.repos[0].bare_cloned);
    assert!(report.value.repos[0].worktree);
}

#[test]
fn status_reports_a_never_synced_repo_as_degraded() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let report = status(&ctx).unwrap();

    assert_eq!(report.value.health, "degraded");
    assert!(!report.value.repos[0].bare_cloned);
}

// -- doctor ---------------------------------------------------------------

#[test]
fn doctor_finds_nothing_in_a_healthy_hall() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(report.value.findings.is_empty());
}

#[test]
fn doctor_names_a_missing_bare_clone_and_its_fix() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root);

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "repo.bare_missing");
    assert!(finding.fix.contains("ivar sync"));
}

// -- doctor: shipped workflow commands ------------------------------------

fn finding<'a>(report: &'a DoctorOutcome, code: &str) -> &'a Diagnosis {
    report
        .findings
        .iter()
        .find(|finding| finding.code == code)
        .unwrap_or_else(|| panic!("no `{code}` finding in {:?}", report.findings))
}

#[test]
fn doctor_reports_missing_shipped_command() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::remove_file(&root.join(".claude/commands/ivar-plan.md")).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "provider.command_missing");
    assert!(finding.what.contains("plan"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_reports_modified_shipped_command() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::write_text(&root.join(".claude/commands/ivar-sync.md"), "tampered\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "provider.command_modified");
    assert!(finding.what.contains("sync"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_reports_modified_legacy_command_with_a_preserve_fix() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::write_text(&root.join(".claude/commands/repo-list.md"), "customised\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "provider.legacy_command_modified");
    assert!(
        finding.what.contains("repo-list.md"),
        "was: {}",
        finding.what
    );
    assert!(
        finding.fix.contains("rename") || finding.fix.contains("remove"),
        "the legacy fix must tell the user to rename or remove the preserved file: {}",
        finding.fix
    );
}

#[test]
fn doctor_reports_stale_commands_for_unavailable_provider() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    // Add OpenCode, sync (materialises its commands), then drop it again.
    let layout = Layout::at(root.clone());
    let both = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &both).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    let claude_only = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &claude_only).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "provider.command_stale");
    assert!(finding.what.contains("opencode"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_says_nothing_about_healthy_commands() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(
        report
            .value
            .findings
            .iter()
            .all(|finding| !finding.code.starts_with("provider.command")),
        "healthy commands must produce no command findings: {:?}",
        report.value.findings
    );
}

// -- init: canonical instructions and the first alias ---------------------

#[test]
fn init_creates_hall_md_and_the_selected_providers_relative_alias() {
    // Claude (the default): HALL.md and a relative CLAUDE.md symlink.
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    let report = init(&ctx, fresh_input()).unwrap();

    assert!(report.is_clean());
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
    assert_eq!(
        fs::read_symlink(&root.join("CLAUDE.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md")),
        "the first alias must be a relative symlink to HALL.md"
    );
    assert!(!fs::exists(&root.join("AGENTS.md")).unwrap());

    // OpenCode: AGENTS.md, and no CLAUDE.md anywhere.
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    let report = init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
    assert_eq!(
        fs::read_symlink(&root.join("AGENTS.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
    assert!(!fs::exists(&root.join("CLAUDE.md")).unwrap());
}

#[test]
fn init_warns_but_stays_valid_when_the_alias_path_is_occupied() {
    let (_guard, root) = utf8_temp_dir();
    fs::write_text(&root.join("CLAUDE.md"), "legacy, precious\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = init(&ctx, fresh_input()).unwrap();

    assert!(!report.is_clean());
    assert_eq!(report.warnings[0].code, "instructions.adoption_required");
    assert_eq!(
        fs::read_text(&root.join("CLAUDE.md")).unwrap().unwrap(),
        "legacy, precious\n",
        "an occupied enabled alias must never be overwritten"
    );
    // The manifest is still valid, and the canonical file still landed.
    let layout = Layout::at(root.clone());
    assert!(Manifest::read(&layout).unwrap().is_some());
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
}

// -- doctor: root instruction topology ------------------------------------

#[test]
fn doctor_names_missing_canonical_instructions() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::remove_file(&root.join("HALL.md")).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.canonical_missing");
    assert!(finding.what.contains("HALL.md"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_a_non_regular_canonical_file() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::remove_file(&root.join("HALL.md")).unwrap();
    fs::ensure_dir(&root.join("HALL.md")).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.canonical_not_regular");
    assert!(finding.fix.contains("regular file"), "was: {}", finding.fix);
}

#[test]
fn doctor_names_a_missing_managed_block() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::write_text(&root.join("HALL.md"), "# House rules\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.managed_block_missing");
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_a_stale_managed_block() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    // Declare a repo after init: the on-disk block was built with none.
    let layout = Layout::at(root.clone());
    let origin = crate::test_support::seeded_repo(
        &root.parent().unwrap().join("origins").join("api"),
        "main",
    );
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![crate::store::manifest::Repo::new(
            crate::domain::name::RepoName::new("api").unwrap(),
            origin.as_str(),
            crate::domain::name::BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.managed_block_stale");
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_a_missing_enabled_alias() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    // Enable OpenCode without ever syncing: its alias is absent.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.alias_missing");
    assert!(finding.what.contains("AGENTS.md"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_an_enabled_regular_alias_with_the_adoption_checklist() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    fs::write_text(&root.join("CLAUDE.md"), "legacy, precious\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.alias_regular");
    assert!(
        finding.fix.contains("HALL.md") && finding.fix.contains("ivar sync"),
        "the adoption checklist must name HALL.md and sync: {}",
        finding.fix
    );
}

#[test]
fn doctor_names_a_broken_alias() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    fs::create_symlink(
        camino::Utf8Path::new("vanished.md"),
        &root.join("CLAUDE.md"),
    )
    .unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.alias_broken");
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_a_wrong_target_alias() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    fs::write_text(&root.join("other.md"), "x").unwrap();
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    fs::create_symlink(camino::Utf8Path::new("other.md"), &root.join("CLAUDE.md")).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.alias_wrong_target");
    assert!(finding.fix.contains("HALL.md"), "was: {}", finding.fix);
}

#[test]
fn doctor_names_a_disabled_providers_leftover_alias() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    let layout = Layout::at(root.clone());
    let both = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &both).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    let claude_only = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &claude_only).unwrap();
    // Replace the leftover symlink with a regular file — the worst case.
    fs::remove_file(&root.join("AGENTS.md")).unwrap();
    fs::write_text(&root.join("AGENTS.md"), "regular file\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.disabled_alias_present");
    assert!(
        finding.fix.contains("regular file"),
        "the finding must say sync removes regular files: {}",
        finding.fix
    );
}

/// Every applicable drift in one run: a stale canonical block, an enabled
/// regular alias, and a broken enabled alias — one `doctor` call reports all
/// of them.
#[test]
fn doctor_returns_every_applicable_instruction_finding_in_one_run() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, fresh_input()).unwrap();
    // Stale block: declare a repo the on-disk block does not list.
    let layout = Layout::at(root.clone());
    let origin = crate::test_support::seeded_repo(
        &root.parent().unwrap().join("origins").join("api"),
        "main",
    );
    let both = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![crate::store::manifest::Repo::new(
            crate::domain::name::RepoName::new("api").unwrap(),
            origin.as_str(),
            crate::domain::name::BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &both).unwrap();
    // CLAUDE.md: an enabled regular alias. AGENTS.md: a broken symlink.
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    fs::write_text(&root.join("CLAUDE.md"), "legacy, precious\n").unwrap();
    fs::create_symlink(
        camino::Utf8Path::new("vanished.md"),
        &root.join("AGENTS.md"),
    )
    .unwrap();

    let report = doctor(&ctx).unwrap();

    let codes: Vec<&str> = report
        .value
        .findings
        .iter()
        .map(|finding| finding.code)
        .collect();
    for expected in [
        "instructions.managed_block_stale",
        "instructions.alias_regular",
        "instructions.alias_broken",
    ] {
        assert!(codes.contains(&expected), "missing {expected} in {codes:?}");
    }
}

// -- doctor: in-flight run receipts ---------------------------------------

#[test]
fn doctor_names_an_active_run_whose_coordinating_session_is_gone() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let feature = FeatureName::new("checkout").unwrap();
    RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        feature.clone(),
        "plans/checkout/plan.md",
        "fingerprint",
        RunBaseline::default(),
        SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    )
    .write(&layout)
    .unwrap();
    // No session View Dir exists — the coordinator is dead.

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "execute.run_orphaned");
    assert!(finding.what.contains("checkout"), "was: {}", finding.what);
    assert!(
        finding.fix.contains("--resume") && finding.fix.contains("--restart"),
        "the fix must offer resume and restart: {}",
        finding.fix
    );
}

#[test]
fn doctor_says_nothing_about_an_active_run_with_a_live_session() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    feature_create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    let feature = FeatureName::new("checkout").unwrap();
    let session = SessionId::new("00000000-0000-0000-0000-000000000002").unwrap();
    RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        feature.clone(),
        "plans/checkout/plan.md",
        "fingerprint",
        RunBaseline::default(),
        session.clone(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    )
    .write(&layout)
    .unwrap();
    fs::ensure_dir(&layout.feature_session(&feature, &session)).unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(
        report
            .value
            .findings
            .iter()
            .all(|finding| finding.code != "execute.run_orphaned"),
        "a live coordinator must not be reported orphaned: {:?}",
        report.value.findings
    );
}

// -- cleanup --------------------------------------------------------------

#[test]
fn cleanup_in_a_non_tty_run_keeps_everything() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    // A repo dir for a repo that is no longer in the manifest.
    let stale = root.join(".ivar/repos/old");
    fs::ensure_dir(&stale).unwrap();

    let report = cleanup(&ctx).unwrap();

    // Non-tty: nothing is deleted without a human.
    assert!(report.value.removed.is_empty());
    assert_eq!(report.value.kept.len(), 1);
    assert!(fs::is_dir(&stale).unwrap());
}

#[test]
fn cleanup_leaves_repos_still_in_the_manifest_alone() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let report = cleanup(&ctx).unwrap();

    assert!(report.value.removed.is_empty());
    assert!(report.value.kept.is_empty());
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
}

#[test]
fn the_human_surface_of_status_names_the_health() {
    let outcome = StatusOutcome {
        root: Utf8PathBuf::from("/hall"),
        health: "operational",
        repos: vec![RepoStatusEntry {
            name: crate::domain::name::RepoName::new("api").unwrap(),
            bare_cloned: true,
            worktree: true,
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Hall at /hall — operational\n  api  cloned  worktree ok\n"
    );
}
