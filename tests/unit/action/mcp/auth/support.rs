#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{AuthMethod, Preregistration, ProviderRun};
use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// Declare one MCP server on the seeded hall's manifest and write it back.
pub(super) fn declare_server(root: &camino::Utf8PathBuf, server: McpServerDef) {
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let updated = manifest.with_mcp_servers(vec![server]).unwrap();
    Manifest::write(&layout, &updated).unwrap();
}

pub(super) fn ok_run(provider: Provider) -> ProviderRun {
    ProviderRun {
        provider,
        preregistration: Preregistration::NotNeeded,
        command: format!("{} mcp auth acme-figma", provider.id()),
        auth_method: AuthMethod::ProviderCommand,
        authenticated: true,
        error: None,
    }
}

pub(super) fn failed_run(provider: Provider, error: &str) -> ProviderRun {
    ProviderRun {
        provider,
        preregistration: Preregistration::NotNeeded,
        command: format!("{} mcp auth acme-figma", provider.id()),
        auth_method: AuthMethod::ProviderCommand,
        authenticated: false,
        error: Some(error.to_owned()),
    }
}
