use crate::domain::mcp::McpServerDef;

pub(crate) const ROOT_KEY: &str = "mcpServers";

pub(crate) fn server_doc(_name: &str, server: &McpServerDef) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("type".to_owned(), serde_json::json!(server.type_));
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
