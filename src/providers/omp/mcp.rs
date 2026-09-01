use crate::domain::mcp::{McpServerDef, McpTransport};

pub(crate) const ROOT_KEY: &str = "mcpServers";

/// OMP's spelling: canonical `http` stays `http`, canonical `local` becomes
/// `stdio`.
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
        // R-MCP-CONFIG: URL must be preserved byte-for-byte including query strings.
        object.insert("url".to_owned(), serde_json::json!(url));
    }
    if let Some(env) = &server.env {
        object.insert("env".to_owned(), serde_json::json!(env));
    }
    if let Some(oauth) = &server.oauth
        && let Some(token_url) = &oauth.token_url
    {
        let mut auth = serde_json::Map::new();
        auth.insert("type".to_owned(), serde_json::json!("oauth"));
        auth.insert("clientId".to_owned(), serde_json::json!(oauth.client_id));
        auth.insert(
            "clientSecret".to_owned(),
            serde_json::json!(format!("${{{}}}", oauth.client_secret_env)),
        );
        auth.insert("tokenUrl".to_owned(), serde_json::json!(token_url));
        if let Some(resource) = &oauth.resource {
            auth.insert("resource".to_owned(), serde_json::json!(resource));
        }
        object.insert("auth".to_owned(), serde_json::Value::Object(auth));
    }

    serde_json::Value::Object(object)
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/mcp.rs"]
mod tests;
