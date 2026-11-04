use super::*;

// -- settings and managed artifact materialisation -------------------------

#[test]
fn sync_materialises_settings_and_artifacts_per_provider() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(
        root.join(".claude/settings.json").is_file(),
        ".claude/settings.json must be created for Claude Code"
    );
    assert!(
        root.join(".opencode/plugins/ivar.js").is_file(),
        ".opencode/plugins/ivar.js must be created for OpenCode"
    );
    assert!(
        root.join(".omp/hooks/pre/ivar.js").is_file(),
        ".omp/hooks/pre/ivar.js must be created for OMP"
    );
    // Settings are committed, so they carry no clone-specific hall root.
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_text(&root.join(".claude/settings.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(settings.pointer("/env/IVAR_HALL").is_none(), "{settings}");
}

#[test]
fn sync_approves_the_declared_servers_in_claude_settings() {
    let (_guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .with_mcp_servers(vec![McpServerDef::new("docs", "local").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_text(&root.join(".claude/settings.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["enabledMcpjsonServers"],
        serde_json::json!(["acme-docs"])
    );
}

#[test]
fn sync_removes_artifacts_when_provider_is_not_listed() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    assert!(root.join(".opencode/plugins/ivar.js").is_file());
    assert!(root.join(".omp/hooks/pre/ivar.js").is_file());

    // Drop OpenCode and OMP from the manifest.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    sync(&ctx, &SyncInput::default()).unwrap();

    assert!(
        !root.join(".opencode/plugins/ivar.js").exists(),
        "plugin must be removed when OpenCode is not listed"
    );
    assert!(
        !root.join(".omp/hooks/pre/ivar.js").exists(),
        "hook must be removed when OMP is not listed"
    );
    // Claude Code's settings survive.
    assert!(root.join(".claude/settings.json").is_file());
}

#[test]
fn sync_honours_memory_capture_false_in_the_manifest() {
    let (_guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let off = Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .with_memory_capture(false)
        .unwrap();
    Manifest::write(&layout, &off).unwrap();

    sync(&Ctx::new(root.clone()), &SyncInput::default()).unwrap();

    let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
    assert!(!settings.contains("ivar memory capture"), "{settings}");
}
