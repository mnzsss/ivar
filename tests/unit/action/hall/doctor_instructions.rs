use super::*;

#[rstest]
#[case::doctor_reports_missing_shipped_command(
    ".claude/commands/ivar-review.md",
    "provider.command_missing",
    "review"
)]
#[case::doctor_reports_missing_shipped_skill(
    ".claude/skills/ivar-execute",
    "provider.skill_missing",
    "execute"
)]
#[case::doctor_names_missing_canonical_instructions(
    "HALL.md",
    "instructions.canonical_missing",
    "HALL.md"
)]
fn doctor_reports_a_removed_managed_file(
    #[case] removed: &str,
    #[case] code: &str,
    #[case] named: &str,
) {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    fs::remove_path(&root.join(removed)).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, code);
    assert!(finding.what.contains(named), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[rstest]
#[case::command(
    ".claude/commands/ivar-sync.md",
    "tampered\n",
    "provider.command_modified",
    "sync"
)]
#[case::skill(
    ".claude/skills/ivar-execute/SKILL.md",
    "tampered skill\n",
    "provider.skill_modified",
    "execute"
)]
fn doctor_reports_a_modified_shipped_file(
    #[case] path: &str,
    #[case] content: &str,
    #[case] code: &str,
    #[case] named: &str,
) {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    fs::write_text(&root.join(path), content).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, code);
    assert!(finding.what.contains(named), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_reports_modified_legacy_command_with_a_preserve_fix() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
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

#[rstest]
#[case::commands("provider.command_stale")]
#[case::skills("provider.skill_stale")]
fn doctor_reports_stale_files_for_unavailable_provider(#[case] code: &str) {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    // Add OpenCode, sync (materialises its commands and skills), then drop it again.
    let layout = Layout::at(root.clone());
    let both = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &both).unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    let claude_only = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &claude_only).unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, code);
    assert!(finding.what.contains("opencode"), "was: {}", finding.what);
    assert!(finding.fix.contains("ivar sync"));
}

#[rstest]
#[case::commands("provider.command")]
#[case::skills("provider.skill")]
fn doctor_says_nothing_about_healthy_files(#[case] prefix: &str) {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(
        report
            .value
            .findings
            .iter()
            .all(|finding| !finding.code.starts_with(prefix)),
        "healthy files must produce no {prefix} findings: {:?}",
        report.value.findings
    );
}

// -- doctor: root instruction topology ------------------------------------

#[test]
fn doctor_names_a_non_regular_canonical_file() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
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
    init(&ctx, &fresh_input()).unwrap();
    fs::write_text(&root.join("HALL.md"), "# House rules\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let finding = finding(&report.value, "instructions.managed_block_missing");
    assert!(finding.fix.contains("ivar sync"));
}

#[test]
fn doctor_names_a_stale_managed_block() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
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
    init(&ctx, &fresh_input()).unwrap();
    // Enable OpenCode without ever syncing: its alias is absent.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
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
    init(&ctx, &fresh_input()).unwrap();
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
    init(&ctx, &fresh_input()).unwrap();
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
    init(&ctx, &fresh_input()).unwrap();
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
    init(&ctx, &fresh_input()).unwrap();
    let layout = Layout::at(root.clone());
    let both = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &both).unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    let claude_only = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
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
    init(&ctx, &fresh_input()).unwrap();
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
