#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

fn stdio_server() -> McpServerDef {
    McpServerDef::new("docs", "stdio")
        .command("npx")
        .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
}

// -- construction --------------------------------------------------------

#[test]
fn a_fresh_definition_carries_only_name_and_type() {
    let def = McpServerDef::new("docs", "stdio");

    assert_eq!(def.name, "docs");
    assert_eq!(def.type_, "stdio");
    assert_eq!(def.command, None);
    assert_eq!(def.args, None);
    assert_eq!(def.url, None);
    assert_eq!(def.env, None);
    assert_eq!(def.oauth, None);
}

#[test]
fn the_setters_fill_the_optional_halves() {
    let def = stdio_server().env(BTreeMap::from([(
        "TOKEN".to_owned(),
        "{env:TOKEN}".to_owned(),
    )]));

    assert_eq!(def.command.as_deref(), Some("npx"));
    assert_eq!(
        def.args,
        Some(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
    );
    assert_eq!(def.url, None);
    assert_eq!(
        def.env.unwrap().get("TOKEN").map(String::as_str),
        Some("{env:TOKEN}")
    );
}

// -- serde: `type` is the wire name, absent halves stay absent -----------

#[test]
fn the_transport_serialises_under_the_key_type() {
    let def = stdio_server();

    let rendered = serde_json::to_value(&def).unwrap();
    assert_eq!(rendered["type"], "stdio");
    assert!(
        rendered.get("command").is_some(),
        "command must be present when set"
    );
    assert!(
        rendered.get("url").is_none(),
        "absent fields must not appear in the JSON"
    );
}

#[test]
fn a_stdio_definition_round_trips_through_serde() {
    let def = stdio_server();

    let parsed: McpServerDef = serde_json::from_value(serde_json::to_value(&def).unwrap()).unwrap();

    assert_eq!(parsed, def);
}

#[test]
fn a_remote_definition_round_trips_with_a_url() {
    let def = McpServerDef::new("sentry", "sse").url("https://mcp.example.com/mcp");

    let parsed: McpServerDef = serde_json::from_value(serde_json::to_value(&def).unwrap()).unwrap();

    assert_eq!(parsed, def);
    assert_eq!(parsed.url.as_deref(), Some("https://mcp.example.com/mcp"));
}

#[test]
fn an_unknown_field_in_a_definition_is_refused() {
    let raw = r#"{"name":"docs","type":"stdio","bogus":true}"#;
    assert!(serde_json::from_str::<McpServerDef>(raw).is_err());
}

// -- oauth: absent by default, a reference to a secret never the value ----

#[test]
fn a_definition_with_no_oauth_omits_the_key_entirely() {
    let def = stdio_server();

    let rendered = serde_json::to_value(&def).unwrap();
    assert!(
        rendered.get("oauth").is_none(),
        "a server with no oauth must not even carry the key"
    );
}

#[test]
fn a_definition_with_oauth_round_trips_and_carries_only_id_and_env_name() {
    let def = McpServerDef::new("figma", "sse")
        .url("https://mcp.figma.com/mcp")
        .oauth(McpOauth::new("client-123", "IVAR_MCP_ACME_FIGMA_SECRET"));

    let rendered = serde_json::to_value(&def).unwrap();
    assert_eq!(rendered["oauth"]["client_id"], "client-123");
    assert_eq!(
        rendered["oauth"]["client_secret_env"],
        "IVAR_MCP_ACME_FIGMA_SECRET"
    );
    // No key on the wire could ever hold a secret value — the struct simply
    // has no field for one.
    assert_eq!(rendered["oauth"].as_object().unwrap().len(), 2);

    let parsed: McpServerDef = serde_json::from_value(rendered).unwrap();
    assert_eq!(parsed, def);
}

#[test]
fn an_unknown_field_on_oauth_is_refused() {
    let raw = r#"{"name":"figma","type":"sse","url":"https://mcp.figma.com/mcp","oauth":{"client_id":"x","client_secret_env":"Y","client_secret":"leaked"}}"#;
    assert!(serde_json::from_str::<McpServerDef>(raw).is_err());
}

// -- materialised_name: the provider-boundary form, never a mutation -----

#[test]
fn materialised_name_prefixes_the_hall_and_leaves_name_untouched() {
    let def = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let hall = HallName::new("acme").unwrap();

    assert_eq!(def.materialised_name(&hall), "acme-figma");
    assert_eq!(def.name, "figma");
}
