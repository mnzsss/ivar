use crate::domain::mcp::{McpServerDef, McpTransport};

pub(crate) const ROOT_KEY: &str = "mcpServers";

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
    }
    if let Some(env) = &server.env {
        object.insert("env".to_owned(), serde_json::json!(env));
    }
    serde_json::Value::Object(object)
}
