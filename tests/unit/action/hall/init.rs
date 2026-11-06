use super::*;

#[test]
fn init_creates_the_expected_on_disk_shape() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());

    let report = init(&ctx, &fresh_input()).unwrap();

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

    let report = init(&ctx, &fresh_input()).unwrap();

    let expected_name = root.file_name().unwrap();
    assert_eq!(report.value.name.as_str(), expected_name);
}

#[test]
fn init_defaults_the_provider_to_claude_code() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let report = init(&ctx, &fresh_input()).unwrap();

    assert_eq!(report.value.provider, Provider::ClaudeCode);
}

#[test]
fn init_honours_an_explicit_name_and_provider() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let report = init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
            git: false,
            origin: None,
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

    init(&ctx, &fresh_input()).unwrap();
    let error = init(&ctx, &fresh_input()).unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "hall.already_initialised");
    assert!(!error.fix_actions.is_empty());
}

#[test]
fn init_rejects_nesting_inside_an_existing_hall() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();

    let nested = root.join("nested");
    fs::ensure_dir(&nested).unwrap();
    let error = init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("nested"),
            name: None,
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "hall.nested");
    assert!(
        error
            .actual
            .as_deref()
            .unwrap()
            .contains("act on that hall")
    );
    assert_eq!(
        error.fix_actions[0].command.as_deref(),
        Some("cd \"$(mktemp -d)\" && ivar init")
    );
}

#[test]
fn init_rejects_an_invalid_explicit_name() {
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root);

    let error = init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("../etc".to_owned()),
            provider: None,
            git: false,
            origin: None,
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

    let error = init(&ctx, &fresh_input()).unwrap_err();

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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: Some("claude".to_owned()),
            git: false,
            origin: None,
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

    init(&ctx, &fresh_input()).unwrap();

    let content = fs::read_text(&root.join(".gitignore")).unwrap().unwrap();
    assert_eq!(
        content,
        ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n!.ivar/scripts/\n\
             .claude/commands/ivar-*.md\n.opencode/commands/ivar-*.md\n.omp/commands/ivar-*.md\n\
             .claude/skills/\n.opencode/skills/\n.omp/skills/\n.tmp/\n"
    );
    assert!(!content.lines().any(|line| line == ".ivar/"));
}

#[test]
fn gitignore_preserves_existing_content_and_does_not_duplicate_on_rerun() {
    let (_guard, root) = utf8_temp_dir();
    fs::write_text(&root.join(".gitignore"), "node_modules/\n").unwrap();
    let ctx = Ctx::new(root.clone());

    init(&ctx, &fresh_input()).unwrap();

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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
            git: false,
            origin: None,
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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
            git: false,
            origin: None,
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
        git: false,
        origin: None,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Initialised hall `acme` at /hall (provider: claude-code)\n"
    );
}

#[test]
fn write_human_reports_the_git_repository_and_origin() {
    let outcome = InitOutcome {
        root: Utf8PathBuf::from("/hall"),
        name: HallName::new("acme").unwrap(),
        provider: Provider::ClaudeCode,
        git: true,
        origin: Some("git@example.com:acme/hall.git".to_owned()),
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Initialised hall `acme` at /hall (provider: claude-code)\nGit repository ready (origin: git@example.com:acme/hall.git)\n"
    );
}

#[test]
fn init_without_git_flags_leaves_the_directory_alone() {
    let (_guard, root) = hall_root();

    let report = init(&Ctx::new(root.clone()), &fresh_input()).unwrap();

    assert!(!root.join(".git").exists());
    assert!(!report.value.git);
    assert_eq!(report.value.origin, None);
}

#[test]
fn init_origin_creates_a_repository_on_main_with_that_origin() {
    let (_guard, root) = hall_root();

    let report = init(
        &Ctx::new(root.clone()),
        &init_with(".", false, Some(ORIGIN)),
    )
    .unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert!(report.value.git);
    assert_eq!(report.value.origin.as_deref(), Some(ORIGIN));
    assert_eq!(git_out(&root, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(git_out(&root, &["remote", "get-url", "origin"]), ORIGIN);
    let json = serde_json::to_value(&report.value).unwrap();
    assert_eq!(json["git"], true);
    assert_eq!(json["origin"], ORIGIN);
}

#[test]
fn init_git_without_url_creates_a_repository_with_no_origin() {
    let (_guard, root) = hall_root();

    let report = init(&Ctx::new(root.clone()), &init_with(".", true, None)).unwrap();

    assert!(report.value.git);
    assert_eq!(report.value.origin, None);
    assert_eq!(git_out(&root, &["remote"]), "");
}

#[test]
fn init_origin_repeated_with_the_same_url_is_idempotent() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &init_with(".", false, Some(ORIGIN))).unwrap();

    let report = init(&ctx, &init_with(".", false, Some(ORIGIN))).unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert!(report.value.git);
    assert_eq!(report.value.origin.as_deref(), Some(ORIGIN));
    assert_eq!(git_out(&root, &["remote"]), "origin");
}

#[test]
fn init_origin_repeated_completes_a_hall_that_was_made_without_git() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();
    let manifest_before = std::fs::read_to_string(root.join("ivar.json")).unwrap();

    let report = init(
        &ctx,
        &InitInput {
            name: None,
            ..init_with(".", false, Some(ORIGIN))
        },
    )
    .unwrap();

    assert!(report.is_clean(), "{:?}", report.warnings);
    assert_eq!(git_out(&root, &["remote", "get-url", "origin"]), ORIGIN);
    assert_eq!(
        std::fs::read_to_string(root.join("ivar.json")).unwrap(),
        manifest_before
    );
}

#[test]
fn init_repeated_without_git_flags_is_still_refused() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &init_with(".", false, Some(ORIGIN))).unwrap();

    let error = init(&ctx, &fresh_input()).unwrap_err();

    assert_eq!(error.code, "hall.already_initialised");
}

#[test]
fn init_origin_with_a_different_url_is_blocked_and_leaves_origin_intact() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &init_with(".", false, Some(ORIGIN))).unwrap();

    let error = init(
        &ctx,
        &init_with(".", false, Some("git@example.com:other/hall.git")),
    )
    .unwrap_err();

    assert_eq!(error.status, Status::Blocked);
    assert_eq!(error.code, "hall.origin_conflict");
    assert!(error.what.contains(ORIGIN), "{}", error.what);
    assert!(
        error.actual.as_deref().unwrap_or_default().contains(ORIGIN),
        "{error:?}"
    );
    assert!(
        error
            .expected
            .as_deref()
            .unwrap_or_default()
            .contains("git@example.com:other/hall.git"),
        "{error:?}"
    );
    assert_eq!(git_out(&root, &["remote", "get-url", "origin"]), ORIGIN);
}

#[test]
fn init_origin_conflict_on_a_new_hall_writes_nothing() {
    let (_guard, root) = hall_root();
    git_out(&root, &["init", "-q", "-b", "main"]);
    git_out(&root, &["remote", "add", "origin", ORIGIN]);

    let error = init(
        &Ctx::new(root.clone()),
        &init_with(".", false, Some("git@example.com:other/hall.git")),
    )
    .unwrap_err();

    assert_eq!(error.code, "hall.origin_conflict");
    assert!(!root.join("ivar.json").exists());
    assert!(!root.join(".ivar").exists());
    assert_eq!(git_out(&root, &["remote", "get-url", "origin"]), ORIGIN);
}

#[test]
fn init_in_an_existing_repository_does_not_reinitialise_it() {
    let (_guard, root) = hall_root();
    git_out(&root, &["init", "-q", "-b", "trunk"]);

    let report = init(
        &Ctx::new(root.clone()),
        &init_with(".", false, Some(ORIGIN)),
    )
    .unwrap();

    assert!(report.value.git);
    assert_eq!(
        git_out(&root, &["symbolic-ref", "--short", "HEAD"]),
        "trunk"
    );
    assert_eq!(git_out(&root, &["remote", "get-url", "origin"]), ORIGIN);
}

#[test]
fn init_in_a_subdirectory_of_a_repository_does_not_run_git_init() {
    let (_guard, root) = hall_root();
    git_out(&root, &["init", "-q", "-b", "main"]);
    let hall = root.join("hall");

    let report = init(&Ctx::new(root.clone()), &init_with("hall", true, None)).unwrap();

    assert!(report.value.git);
    assert!(!hall.join(".git").exists());
}

#[rstest]
#[case("")]
#[case("   ")]
#[case("--upload-pack=evil")]
fn init_rejects_an_unusable_origin_before_writing(#[case] url: &str) {
    let (_guard, root) = hall_root();

    let error = init(&Ctx::new(root.clone()), &init_with(".", false, Some(url))).unwrap_err();

    assert_eq!(error.code, "hall.origin_invalid");
    assert!(!root.join("ivar.json").exists());
}

// -- init: canonical instructions and the first alias ---------------------

#[test]
fn init_creates_hall_md_and_the_selected_providers_relative_alias() {
    // Claude (the default): HALL.md and a relative CLAUDE.md symlink.
    let (_guard, root) = utf8_temp_dir();
    let ctx = Ctx::new(root.clone());
    let report = init(&ctx, &fresh_input()).unwrap();

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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
            git: false,
            origin: None,
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

    let report = init(&ctx, &fresh_input()).unwrap();

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
