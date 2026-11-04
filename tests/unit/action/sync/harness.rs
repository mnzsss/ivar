use super::*;

#[test]
fn sync_materialises_shipped_commands_for_available_providers() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    for provider in Provider::ALL {
        let dir = root.join(provider.commands_dir());
        for command in commands::catalog() {
            assert!(
                fs::is_file(&dir.join(command.file_name())).unwrap(),
                "{} missing for {provider}",
                command.file_name()
            );
        }
    }
    // OpenCode's commands come from this sync — init only bootstrapped the
    // default provider, Claude Code.
    assert_eq!(
        entry(&report.value, "opencode", "command ivar-review.md").change,
        Change::Created
    );
}

#[test]
fn sync_materialises_shipped_skills_for_available_providers() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    for provider in Provider::ALL {
        let skill_path = root
            .join(provider.skills_dir())
            .join("ivar-execute/SKILL.md");
        assert!(
            fs::is_file(&skill_path).unwrap(),
            "Skill must exist at {skill_path}"
        );
    }
    assert_eq!(
        entry(&report.value, "opencode", "skill ivar-execute").change,
        Change::Created
    );
}

#[test]
fn second_sync_reports_commands_unchanged_without_rewriting() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    let dir = root.join(".claude/commands");
    let before: Vec<(Utf8PathBuf, Vec<u8>, Option<std::time::SystemTime>)> = commands::catalog()
        .iter()
        .map(|command| {
            let path = dir.join(command.file_name());
            let bytes = fs::read_bytes(&path).unwrap().unwrap();
            let mtime = std::fs::metadata(path.as_std_path())
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            (path, bytes, mtime)
        })
        .collect();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    for (path, before_bytes, before_mtime) in &before {
        assert_eq!(
            fs::read_bytes(path).unwrap().unwrap(),
            *before_bytes,
            "{path}"
        );
        let mtime = std::fs::metadata(path.as_std_path())
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        assert_eq!(&mtime, before_mtime, "{path} must not be rewritten");
    }
    assert_eq!(
        entry(&report.value, "claude-code", "command ivar-review.md").change,
        Change::Unchanged
    );
}

#[test]
fn sync_repairs_modified_shipped_command_and_preserves_custom_command() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    let custom = root.join(".claude/commands/custom.md");
    fs::write_text(&custom, "mine\n").unwrap();
    fs::write_text(&root.join(".claude/commands/ivar-review.md"), "changed\n").unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "claude-code", "command ivar-review.md").change,
        Change::Updated
    );
    assert_eq!(
        fs::read_text(&root.join(".claude/commands/ivar-review.md"))
            .unwrap()
            .unwrap(),
        embedded("review")
    );
    assert_eq!(fs::read_text(&custom).unwrap().unwrap(), "mine\n");
}

#[test]
fn sync_removes_only_shipped_commands_for_unavailable_provider() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    let custom = root.join(".opencode/commands/custom.md");
    fs::write_text(&custom, "mine\n").unwrap();

    // Drop OpenCode from the manifest.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "command ivar-review.md").change,
        Change::Removed
    );
    assert!(
        !fs::exists(&root.join(".opencode/commands/ivar-review.md")).unwrap(),
        "a dropped provider's shipped commands must be removed"
    );
    assert_eq!(
        fs::read_text(&custom).unwrap().unwrap(),
        "mine\n",
        "the user's command must survive provider removal"
    );
}

/// Deterministic by construction: a regular file occupying the parent path
/// refuses `ensure_dir` regardless of whether the test runs as root —
/// permission bits would not be.
#[test]
fn command_write_failure_warns_and_other_provider_steps_continue() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(&root.join(".opencode"), "not a directory\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(
        !report.is_clean(),
        "a failed command write must not be clean"
    );
    assert!(
        report.value.entries.iter().any(|e| e.surface == "opencode"
            && e.label == "official commands"
            && e.change == Change::Failed),
        "expected a failed opencode commands entry in {:?}",
        report.value.entries
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.subject == "opencode"),
        "expected an opencode warning in {:?}",
        report.warnings
    );
    // The other provider's commands and config completed regardless.
    assert!(fs::is_file(&root.join(".claude/commands/ivar-review.md")).unwrap());
    assert!(fs::is_file(&root.join("CLAUDE.md")).unwrap());
    // OpenCode's own non-command config still landed.
    assert!(fs::is_file(&root.join("AGENTS.md")).unwrap());
    assert!(fs::is_file(&root.join("opencode.json")).unwrap());
}

// -- root instruction topology --------------------------------------------

#[test]
fn sync_repairs_absent_and_wrong_enabled_symlinks() {
    let (_guard, root) = hall_with_all_providers();
    // Break both aliases: remove CLAUDE.md, point AGENTS.md elsewhere.
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    fs::write_text(&root.join("other.md"), "x").unwrap();
    fs::create_symlink(Utf8Path::new("other.md"), &root.join("AGENTS.md")).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "claude-code", "CLAUDE.md alias").change,
        Change::Created
    );
    assert_eq!(
        entry(&report.value, "providers", "AGENTS.md alias").change,
        Change::Updated
    );
    assert_eq!(
        fs::read_symlink(&root.join("CLAUDE.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
    assert_eq!(
        fs::read_symlink(&root.join("AGENTS.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
}

#[test]
fn sync_preserves_an_enabled_regular_alias_and_reports_conflict() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(&root.join("AGENTS.md"), "legacy, precious\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(!report.is_clean());
    assert_eq!(
        entry(&report.value, "providers", "AGENTS.md alias").change,
        Change::Failed
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "instructions.adoption_required"),
        "warnings: {:?}",
        report.warnings
    );
    assert_eq!(
        fs::read_text(&root.join("AGENTS.md")).unwrap().unwrap(),
        "legacy, precious\n",
        "an enabled regular alias must be preserved byte for byte"
    );
}

#[test]
fn a_conflict_does_not_abort_repo_mcp_or_command_reconciliation() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        Manifest::read(&layout).unwrap().unwrap().repos().to_vec(),
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    fs::write_text(&root.join("AGENTS.md"), "legacy, precious\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(!report.is_clean());
    assert_eq!(
        entry(&report.value, "providers", "AGENTS.md alias").change,
        Change::Failed
    );
    // The repo, the MCP config, and the commands still land.
    assert_eq!(
        entry(&report.value, "repo api", "bare clone").change,
        Change::Created
    );
    assert!(fs::is_file(&root.join(".mcp.json")).unwrap());
    assert!(fs::is_file(&root.join(".opencode/commands/ivar-review.md")).unwrap());
    assert!(fs::is_file(&root.join("CLAUDE.md")).unwrap());
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
}

#[test]
fn removing_a_provider_by_hand_makes_sync_delete_its_regular_alias() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    // Drop OpenCode from the manifest by hand — the destructive
    // disabled-provider rule now authorises deleting its alias entry.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    fs::remove_file(&root.join("AGENTS.md")).unwrap();
    fs::write_text(&root.join("AGENTS.md"), "regular file\n").unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "providers", "AGENTS.md alias").change,
        Change::Removed
    );
    assert!(!fs::exists(&root.join("AGENTS.md")).unwrap());
    assert!(
        fs::is_file(&root.join("HALL.md")).unwrap(),
        "the canonical file must always survive"
    );
}

/// `sync` runs after every `git pull`; a healthy second run must leave file
/// mtimes untouched, or every run dirties `git status`.
#[test]
fn repeated_healthy_sync_leaves_file_mtimes_unchanged() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();

    let paths = [
        root.join("HALL.md"),
        root.join("CLAUDE.md"),
        root.join("AGENTS.md"),
    ];
    let before: Vec<(Utf8PathBuf, Option<std::time::SystemTime>)> = paths
        .iter()
        .map(|path| {
            let mtime = std::fs::metadata(path.as_std_path())
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            (path.clone(), mtime)
        })
        .collect();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    for (path, before_mtime) in &before {
        let mtime = std::fs::metadata(path.as_std_path())
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        assert_eq!(&mtime, before_mtime, "{path} must not be rewritten");
    }
}

#[test]
fn sync_report_groups_shared_alias_surfaces_into_single_entry() {
    // `hall_with` seeds repos, not providers; the provider set lives in the
    // manifest. Write a manifest listing both AGENTS.md owners, then sync.
    let (_guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::OpenCode, Provider::Omp], Provider::OpenCode),
        Vec::new(),
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let ctx = Ctx::new(root.clone());
    let report = sync(&ctx, &SyncInput::default()).unwrap();

    let agents_entries: Vec<_> = report
        .value
        .entries
        .iter()
        .filter(|e| e.label == "AGENTS.md alias")
        .collect();

    assert_eq!(
        agents_entries.len(),
        1,
        "one physical alias must produce one report line, got: {agents_entries:?}"
    );
    assert_eq!(
        agents_entries[0].surface, "providers",
        "a surface owned by several providers is attributed to none of them alone"
    );
}
