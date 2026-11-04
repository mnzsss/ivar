use super::*;

// -- providers -------------------------------------------------------------

#[test]
fn a_disabled_providers_alias_entry_is_removed_entirely() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    // A stale regular AGENTS.md from when the hall did list OpenCode — with
    // the user's own text inside it. The disabled-provider rule removes the
    // whole entry, including a regular file; it never removes HALL.md.
    fs::write_text(
        &root.join("AGENTS.md"),
        &format!(
            "{}\nstale\n{}\n\n# House rules\n",
            config::MANAGED_START,
            config::MANAGED_END
        ),
    )
    .unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "providers", "AGENTS.md alias").change,
        Change::Removed
    );
    assert!(
        !fs::exists(&root.join("AGENTS.md")).unwrap(),
        "a disabled provider's alias path is entirely ivar-managed"
    );
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
}

#[test]
fn a_provider_the_hall_does_not_list_and_never_did_is_unchanged() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "providers", "AGENTS.md alias").change,
        Change::Unchanged
    );
    assert!(!fs::exists(&root.join("AGENTS.md")).unwrap());
}

// -- MCP config -----------------------------------------------------------

#[test]
fn sync_materialises_the_mcp_config_at_the_hall_root() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "claude-code", ".mcp.json MCP config").change,
        Change::Created
    );
    let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
    // Valid JSON, matching the empty-server v1 shape.
    let parsed: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
    assert_eq!(parsed, serde_json::json!({ "mcpServers": {} }));
}

/// `sync` runs after every `git pull`; the second run must leave the MCP
/// config byte-identical too.
#[test]
fn the_mcp_config_is_unchanged_on_a_second_sync() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    let before = fs::read_bytes(&root.join(".mcp.json")).unwrap().unwrap();

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "claude-code", ".mcp.json MCP config").change,
        Change::Unchanged
    );
    assert_eq!(
        fs::read_bytes(&root.join(".mcp.json")).unwrap().unwrap(),
        before
    );
}

#[test]
fn sync_materialises_the_opencode_config_when_opencode_is_available() {
    let (_guard, root) = hall_with(&[]);
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
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "opencode.json MCP config").change,
        Change::Created
    );
    let on_disk = fs::read_text(&root.join("opencode.json")).unwrap().unwrap();
    assert!(on_disk.contains("$schema"), "was: {on_disk}");
    assert!(on_disk.contains("\"mcp\": {}"), "was: {on_disk}");
}

#[test]
fn sync_strips_a_stale_mcp_config_for_a_provider_the_hall_dropped() {
    let (_guard, root) = hall_with(&[]);
    // A stale opencode.json from when the hall did list OpenCode — carrying
    // a user key that must survive the strip.
    fs::write_text(
        &root.join("opencode.json"),
        &serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": { "stale": { "type": "local", "command": ["old"] } },
        })
        .to_string(),
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "opencode.json MCP config").change,
        Change::Removed
    );
    let on_disk = fs::read_text(&root.join("opencode.json")).unwrap().unwrap();
    assert!(on_disk.contains("claude-sonnet-4-5"), "was: {on_disk}");
    assert!(!on_disk.contains("stale"), "was: {on_disk}");
}

#[test]
fn sync_writes_declared_servers_into_the_config() {
    let (_guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_mcp_servers(vec![McpServerDef::new("docs", "local").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
    assert!(on_disk.contains("\"acme-docs\""), "was: {on_disk}");
    assert!(on_disk.contains("\"npx\""), "was: {on_disk}");
}

#[test]
fn sync_with_omp_available_writes_every_provider_mcp_document() {
    let (_guard, root) = hall_with_all_providers();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_mcp_servers(vec![McpServerDef::new("docs", "local").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(report.is_clean());
    let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
    assert!(on_disk.contains("\"acme-docs\""), "was: {on_disk}");

    let omp_doc = fs::read_text(&root.join(".omp/mcp.json")).unwrap().unwrap();
    assert!(omp_doc.contains("\"acme-docs\""), "was: {omp_doc}");
    let parsed: serde_json::Value = serde_json::from_str(&omp_doc).unwrap();
    assert!(
        parsed.get("mcpServers").is_some(),
        "OMP's document hangs servers off `mcpServers`: {omp_doc}"
    );
}

#[test]
fn sync_reports_a_malformed_omp_mcp_document_instead_of_silently_skipping_it() {
    let (_guard, root) = hall_with_all_providers();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_mcp_servers(vec![McpServerDef::new("docs", "local").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    // A root-level array is the shape `materialise_mcp` refuses.
    let omp_mcp = root.join(".omp/mcp.json");
    fs::ensure_dir(omp_mcp.parent().unwrap()).unwrap();
    fs::write_text(&omp_mcp, "[\"not an object\"]").unwrap();

    let ctx = Ctx::new(root.clone());
    let report = sync(&ctx, &SyncInput::default()).unwrap();

    assert!(
        !report.is_clean(),
        "a malformed document must not report clean"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "sync.step_failed" && w.what.contains("is not a JSON object")),
        "warnings were: {:?}",
        report.warnings
    );
}

#[test]
fn sync_removes_the_omp_mcp_document_when_omp_leaves_the_hall() {
    let (_guard, root) = hall_with_all_providers();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_mcp_servers(vec![McpServerDef::new("docs", "local").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, &SyncInput::default()).unwrap();
    assert!(fs::exists(&root.join(".omp/mcp.json")).unwrap());

    // Drop OMP from the hall; its document must go the way `remove_mcp`
    // takes the other providers' documents.
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_providers(Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ))
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    sync(&ctx, &SyncInput::default()).unwrap();
    assert!(
        !fs::exists(&root.join(".omp/mcp.json")).unwrap(),
        "a provider the hall no longer lists keeps no MCP document"
    );
}

#[test]
fn sync_moves_omp_servers_out_of_the_legacy_root_mcp_json() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(
        &root.join("mcp.json"),
        r#"{"mcpServers":{"acme-old":{"type":"http","url":"https://old.example/mcp"}}}"#,
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    assert!(
        !fs::exists(&root.join("mcp.json")).unwrap(),
        "a root mcp.json holding only this hall's servers is removed"
    );
    assert!(fs::exists(&root.join(".omp/mcp.json")).unwrap());
}

#[test]
fn sync_keeps_user_servers_in_the_legacy_root_mcp_json() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(
        &root.join("mcp.json"),
        r#"{"mcpServers":{"acme-old":{"type":"http","url":"https://old.example/mcp"},"mine":{"type":"http","url":"https://mine.example/mcp"}}}"#,
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_text(&root.join("mcp.json")).unwrap().unwrap()).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({
            "mcpServers": { "mine": { "type": "http", "url": "https://mine.example/mcp" } }
        }),
        "only this hall's servers leave the root mcp.json"
    );
}

#[test]
fn sync_keeps_unrelated_keys_in_the_legacy_root_mcp_json() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(
        &root.join("mcp.json"),
        r#"{"mcpServers":{"acme-old":{"type":"http"}},"custom":true}"#,
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    sync(&ctx, &SyncInput::default()).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_text(&root.join("mcp.json")).unwrap().unwrap()).unwrap();
    assert_eq!(parsed, serde_json::json!({ "custom": true }));
}
