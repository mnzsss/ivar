use serde_json::Value;

use crate::domain::capability::provider_for_client_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpClient {
    pub provider: String,
    pub instance: uuid::Uuid,
}

impl McpClient {
    #[must_use]
    pub fn from_initialize(params: &Value, instance: uuid::Uuid) -> Self {
        let provider = params
            .get("clientInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .and_then(provider_for_client_name)
            .map_or_else(|| "unknown".to_owned(), |provider| provider.id().to_owned());
        Self { provider, instance }
    }

    #[must_use]
    pub fn owner_key(&self) -> String {
        format!("{}:mcp-{}", self.provider, self.instance)
    }
}

pub(crate) fn track_client(current: &mut Option<McpClient>, req: &Value, instance: uuid::Uuid) {
    if req.get("method").and_then(Value::as_str) == Some("initialize") {
        *current = Some(McpClient::from_initialize(
            req.get("params").unwrap_or(&Value::Null),
            instance,
        ));
    }
}
