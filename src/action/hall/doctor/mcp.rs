use crate::action::graph::mcp_registration::{LEGACY_SERVER_NAME, runs_graph_mcp};
use crate::domain::provider::Provider;
use crate::providers;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::Diagnosis;

pub(crate) fn mcp_name_diagnoses(layout: &Layout, manifest: &Manifest) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    if manifest
        .mcp_servers()
        .iter()
        .any(|server| server.name == LEGACY_SERVER_NAME && runs_graph_mcp(server))
    {
        findings.push(Diagnosis {
            code: "mcp.legacy_name",
            what: "ivar.json names the graph MCP server `graph`; the server is now `ivar`"
                .to_owned(),
            fix: "Run `ivar sync`.".to_owned(),
        });
    }
    let stale_key = format!("{}-{LEGACY_SERVER_NAME}", manifest.name().as_str());
    for provider in manifest.providers().available() {
        if provider_config_has(layout, *provider, &stale_key) {
            findings.push(Diagnosis {
                code: "mcp.provider_config_stale",
                what: format!(
                    "{} still has the MCP server `{stale_key}`",
                    provider.mcp_config_path()
                ),
                fix: "Run `ivar sync`.".to_owned(),
            });
        }
    }
    findings
}

fn provider_config_has(layout: &Layout, provider: Provider, key: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(layout.mcp_config(&provider)) else {
        return false;
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let Some(entry) = doc
        .get(providers::mcp_root_key(provider))
        .and_then(|servers| servers.get(key))
    else {
        return false;
    };
    let text_at = |value: Option<&serde_json::Value>| {
        value.and_then(serde_json::Value::as_str).map(str::to_owned)
    };
    let (command, first, second) = match entry.get("command") {
        Some(serde_json::Value::Array(parts)) => (
            text_at(parts.first()),
            text_at(parts.get(1)),
            text_at(parts.get(2)),
        ),
        other => {
            let args = entry.get("args").and_then(serde_json::Value::as_array);
            (
                text_at(other),
                text_at(args.and_then(|a| a.first())),
                text_at(args.and_then(|a| a.get(1))),
            )
        }
    };
    let is_ivar = command
        .as_deref()
        .and_then(|c| std::path::Path::new(c).file_stem())
        .is_some_and(|stem| stem == "ivar");
    is_ivar && first.as_deref() == Some("graph") && second.as_deref() == Some("mcp")
}
