use crate::domain::mcp::{McpServerDef, McpTransport};

pub(crate) const ROOT_KEY: &str = "mcpServers";

/// Slack's MCP server offers no dynamic client registration. Anthropic's
/// `productivity` plugin declares this client id and loopback port for
/// `mcp.slack.com`: Slack's partner app for Claude Code, which takes no
/// secret. They identify Claude Code, so other providers must not render them.
const SLACK_ORIGIN: &str = "https://mcp.slack.com";
const SLACK_PARTNER_CLIENT_ID: &str = "1601185624273.8899143856786";
const SLACK_PARTNER_CALLBACK_PORT: u16 = 3118;

fn is_slack(url: &str) -> bool {
    url.strip_prefix(SLACK_ORIGIN)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// Claude Code's spelling: canonical `http` stays `http`, canonical `local`
/// becomes `stdio`. `command`/`args`/`env` are Claude's native shape and are
/// carried through unchanged.
pub(crate) fn server_doc(
    _name: &str,
    server: &McpServerDef,
    transport: McpTransport,
) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    let type_str = match transport {
        McpTransport::Http => "http",
        McpTransport::Local => "stdio",
    };
    object.insert("type".to_owned(), serde_json::json!(type_str));
    if let Some(command) = &server.command {
        object.insert("command".to_owned(), serde_json::json!(command));
    }
    if let Some(args) = &server.args {
        object.insert("args".to_owned(), serde_json::json!(args));
    }
    if let Some(url) = &server.url {
        object.insert("url".to_owned(), serde_json::json!(url));
        if is_slack(url) {
            object.insert(
                "oauth".to_owned(),
                serde_json::json!({
                    "clientId": SLACK_PARTNER_CLIENT_ID,
                    "callbackPort": SLACK_PARTNER_CALLBACK_PORT,
                }),
            );
        }
    }
    if let Some(env) = &server.env {
        object.insert("env".to_owned(), serde_json::json!(env));
    }
    serde_json::Value::Object(object)
}
