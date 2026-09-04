use crate::domain::mcp::McpServerDef;

pub(crate) const ROOT_KEY: &str = "mcp";

pub(crate) const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";

pub(crate) fn server_doc(_name: &str, server: &McpServerDef) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    let transport = if server.type_ == "stdio" {
        "local"
    } else {
        "remote"
    };
    object.insert("type".to_owned(), serde_json::json!(transport));
    if server.type_ == "stdio" {
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
