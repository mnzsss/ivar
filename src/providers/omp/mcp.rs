use crate::domain::mcp::{McpServerDef, McpTransport};
use crate::providers::omp::auth::credential_id;

pub(crate) const ROOT_KEY: &str = "mcpServers";

/// The hall-root MCP file omp also reads; sync removes this hall's servers
/// from it. omp lets `opencode.json`'s `mcp` entries win over this file, so a
/// leftover copy of the hall's servers here is dead config.
pub(crate) const LEGACY_ROOT_CONFIG: &str = "mcp.json";

/// OMP's spelling: canonical `http` stays `http`, canonical `local` becomes
/// `stdio`.
pub(crate) fn server_doc(
    name: &str,
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
    if transport == McpTransport::Http {
        let mut auth = serde_json::Map::new();
        auth.insert("type".to_owned(), serde_json::json!("oauth"));
        // omp resolves `auth.credentialId` verbatim and rejects an id under
        // `mcp_oauth:profile:` unless it is the server's URL key.
        auth.insert(
            "credentialId".to_owned(),
            serde_json::json!(credential_id(name)),
        );
        if let Some(oauth) = &server.oauth
            && let Some(token_url) = &oauth.token_url
        {
            auth.insert("clientId".to_owned(), serde_json::json!(oauth.client_id));
            if let Some(secret_env) = &oauth.client_secret_env {
                auth.insert(
                    "clientSecret".to_owned(),
                    serde_json::json!(format!("${{{secret_env}}}")),
                );
            }
            auth.insert("tokenUrl".to_owned(), serde_json::json!(token_url));
            if let Some(resource) = &oauth.resource {
                auth.insert("resource".to_owned(), serde_json::json!(resource));
            }
        }
        object.insert("auth".to_owned(), serde_json::Value::Object(auth));
    }

    serde_json::Value::Object(object)
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/mcp.rs"]
mod tests;
