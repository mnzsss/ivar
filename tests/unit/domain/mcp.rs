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
