use crate::domain::mcp::{McpServerDef, McpTransport};
/// The loopback callback ivar's own OAuth exchange listens on. Shared, not
/// native: it belongs to the callback listener, so it is consumed from there
/// rather than restated.
use crate::infra::http_callback::OAUTH_REDIRECT_URI;

/// The key OpenCode hangs its MCP servers off, inside `opencode.json`.
///
/// Sourced, not guessed: `docs/wayfinder/bifrost-open-source/BACKLOG.md`
/// item B19 surveys `vibe-kanban` (Apache-2.0, nine working harness
/// adapters) and records `mcp` + `$schema` for OpenCode verbatim.
pub(crate) const ROOT_KEY: &str = "mcp";

/// OpenCode's spelling: canonical `local` becomes `local` with `command` as
/// one array, canonical `http` becomes `remote` with a `url`.
pub(crate) fn server_doc(
    _name: &str,
    server: &McpServerDef,
    transport: McpTransport,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    let type_str = match transport {
        McpTransport::Http => "remote",
        McpTransport::Local => "local",
    };
    object.insert("type".to_owned(), serde_json::json!(type_str));
    if transport == McpTransport::Local {
        let mut command: Vec<&str> = Vec::new();
        if let Some(binary) = &server.command {
            command.push(binary);
        }
        if let Some(args) = &server.args {
            command.extend(args.iter().map(String::as_str));
        }
        if !command.is_empty() {
            object.insert("command".to_owned(), serde_json::json!(command));
        }
    } else if let Some(url) = &server.url {
        object.insert("url".to_owned(), serde_json::json!(url));
    }
    if let Some(env) = &server.env {
        object.insert("environment".to_owned(), serde_json::json!(env));
    }
    // A pre-provisioned OAuth client, for a server whose host rejects
    // OpenCode's own dynamic client registration. `clientSecret` is
    // always the `{env:NAME}` reference the manifest names, never a
    // value — `McpOauth` has no field that could hold one.
    // Claude Code never reaches this: it is on Figma's allowlist and
    // needs no pre-registration (R-SECRET-HANDOFF, R-NO-SECRETS).
    if let Some(oauth) = &server.oauth {
        object.insert(
            "oauth".to_owned(),
            serde_json::json!({
                "clientId": oauth.client_id,
                "clientSecret": format!("{{env:{}}}", oauth.client_secret_env),
                "redirectUri": OAUTH_REDIRECT_URI,
            }),
        );
    }
    serde_json::Value::Object(object)
}
