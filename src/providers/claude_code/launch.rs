use crate::infra::proc::Command;
use crate::providers::{Capabilities, LaunchContract};

const CAPABILITIES: Capabilities = Capabilities {
    supports_resume: true,
    supports_review: true,
    interactive: true,
};

const CONTRACT: LaunchContract = LaunchContract {
    binary: "claude",
    capabilities: CAPABILITIES,
};

#[must_use]
pub const fn contract() -> LaunchContract {
    CONTRACT
}

/// Claude's launch argv.
///
/// `mcp_allowlist` is serialised into `--settings` under the
/// `enabledMcpjsonServers` key, scoping approval to this process only: no
/// settings file is written and no user-global configuration is touched. The
/// flag is never omitted — an empty list is passed explicitly so that a hall
/// declaring no MCP servers approves none.
///
/// The names arrive already sorted and deduplicated; this function passes
/// them through as given.
pub fn start_command(resume: bool, mcp_allowlist: &[String]) -> Command {
    let mut command = Command::new(CONTRACT.binary);
    if resume {
        command = command.arg("--continue");
    }
    let settings = serde_json::json!({ "enabledMcpjsonServers": mcp_allowlist });
    command.arg("--settings").arg(settings.to_string())
}
