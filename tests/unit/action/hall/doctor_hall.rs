use super::*;

// -- doctor ---------------------------------------------------------------

#[test]
fn doctor_finds_nothing_in_a_healthy_hall() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();

    let report = doctor(&ctx).unwrap();

    assert!(report.value.findings.is_empty());
}

#[test]
fn doctor_never_runs_t3_for_a_hall_that_does_not_list_it() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    let fake = FakeT3::install(&root);
    let _t3 = fake_t3_on_this_thread(&fake);

    let report = doctor(&ctx).unwrap();

    assert!(report.value.findings.is_empty());
    assert_eq!(fake.log(), "");
}

#[test]
fn doctor_is_clean_when_t3_answers() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    enable_t3_host(&root, Provider::ClaudeCode);
    let fake = FakeT3::install(&root);
    let _t3 = fake_t3_on_this_thread(&fake);

    let report = doctor(&ctx).unwrap();

    assert!(report.value.findings.is_empty());
    assert!(fake.log().starts_with("--version\nproject remove "));
}

#[test]
fn doctor_reports_an_outdated_t3_and_runs_no_project_command() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    enable_t3_host(&root, Provider::ClaudeCode);
    let fake = FakeT3::install(&root);
    let _t3 =
        crate::test_support::redirect_t3(fake_t3_command(&fake).env("T3_FAKE_VERSION", "0.0.45"));

    let findings = doctor(&ctx).unwrap().value.findings;

    let finding = findings
        .iter()
        .find(|finding| finding.code == "t3.version_unsupported")
        .expect("an outdated t3 is reported");
    assert!(finding.what.contains("v0.0.45"));
    assert!(finding.fix.contains("T3CODE_CLI_PATH"));
    assert_eq!(fake.log(), "--version\n");
}

#[test]
fn doctor_names_an_unreachable_t3_server_and_a_missing_t3_binary() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    enable_t3_host(&root, Provider::ClaudeCode);
    let fake = FakeT3::install(&root);

    {
        let _t3 = crate::test_support::redirect_t3(fake_t3_command(&fake).env("T3_FAKE_FAIL", "1"));
        let findings = doctor(&ctx).unwrap().value.findings;
        let finding = findings
            .iter()
            .find(|finding| finding.code == "t3.server_unreachable")
            .expect("an unanswered probe is reported");
        assert!(
            finding
                .what
                .contains("DesktopAppRequestFailedError: fake failure")
        );
        assert!(finding.fix.contains("Start the T3 Code desktop app"));
    }
    {
        let _gone =
            crate::test_support::redirect_t3(crate::infra::proc::Command::new("/nonexistent/t3"));
        let findings = doctor(&ctx).unwrap().value.findings;
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == "t3.binary_missing")
        );
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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
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
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
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

#[test]
fn doctor_reports_working_docs_in_legacy_locations() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    // Create legacy plans/<feature>/ directory with an SPDD artifact
    let legacy_plan_dir = root.join("plans/checkout");
    fs::ensure_dir(&legacy_plan_dir).unwrap();
    fs::write_text(&legacy_plan_dir.join("plan.md"), "# Plan\n").unwrap();

    // Create legacy docs/<feature>/discovery.md
    let legacy_docs_dir = root.join("docs/checkout");
    fs::ensure_dir(&legacy_docs_dir).unwrap();
    fs::write_text(&legacy_docs_dir.join("discovery.md"), "# Discovery\n").unwrap();

    let report = doctor(&ctx).unwrap();

    let legacy_findings: Vec<_> = report
        .value
        .findings
        .iter()
        .filter(|f| f.code == "hall.working_docs_legacy_location")
        .collect();

    assert!(
        !legacy_findings.is_empty(),
        "expected hall.working_docs_legacy_location findings, got: {:?}",
        report.value.findings
    );
    for finding in &legacy_findings {
        assert!(
            finding.fix.contains(".ivar/features/"),
            "fix must guide user to .ivar/features/: {}",
            finding.fix
        );
    }
}

#[test]
fn doctor_ignores_topic_docs_directories() {
    let (_guard, root) = hall_with_repo();
    let ctx = Ctx::new(root.clone());

    // Topic doc directories under docs/ (product, updates, repo-relations) are not legacy feature docs
    for topic in ["product", "updates", "repo-relations"] {
        let topic_dir = root.join(format!("docs/{topic}"));
        fs::ensure_dir(&topic_dir).unwrap();
        fs::write_text(&topic_dir.join("overview.md"), "# Overview\n").unwrap();
    }

    let report = doctor(&ctx).unwrap();

    assert!(
        report
            .value
            .findings
            .iter()
            .all(|f| f.code != "hall.working_docs_legacy_location"),
        "topic docs must not be flagged as legacy working docs: {:?}",
        report.value.findings
    );
}

#[test]
fn doctor_diagnoses_missing_mcp_auth() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let updated = manifest
        .with_mcp_servers(vec![
            McpServerDef::new("figma", "http")
                .url("https://mcp.figma.com/mcp")
                .oauth(McpOauth::public("cid")),
        ])
        .unwrap();
    Manifest::write(&layout, &updated).unwrap();

    let mock_state =
        |_p: Provider, _name: &str, _url: &str| -> CredentialState { CredentialState::Missing };

    let diagnoses = diagnose_mcp_auth(&updated, &mock_state);
    assert_eq!(diagnoses.len(), updated.providers().available().len());
    assert_eq!(diagnoses[0].code, "mcp.auth_missing");
    assert!(diagnoses[0].what.contains("figma"));
    assert!(diagnoses[0].fix.contains("ivar mcp auth figma --provider"));
}

#[test]
fn doctor_ignores_mcp_servers_not_requiring_auth() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(&ctx, &fresh_input()).unwrap();

    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let updated = manifest
        .with_mcp_servers(vec![
            McpServerDef::new("docs", "local").command("npx"),
            McpServerDef::new("sentry", "http").url("https://sentry.io/mcp"),
        ])
        .unwrap();

    let mock_state =
        |_p: Provider, _name: &str, _url: &str| -> CredentialState { CredentialState::Missing };

    let diagnoses = diagnose_mcp_auth(&updated, &mock_state);
    assert!(diagnoses.is_empty());
}
