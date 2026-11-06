//! What each provider supports among the features that depend on one.
//! Not the launch flags of `crate::providers::Capabilities`: these are
//! hall-memory features (capture, brief, …) that a provider either
//! carries or explains why it does not.

use crate::domain::provider::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Supported,
    NotSupported(&'static str),
}

/// One `Support` field per provider-dependent feature. A new field breaks
/// every literal in [`provider_support`] until each provider declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSupport {
    pub session_started: Support,
    pub session_prompted: Support,
    pub session_tool: Support,
    pub session_ended: Support,
    pub cleanup_command: Support,
    pub plan_memory_step: Support,
    pub memory_mcp: Support,
    pub planning_memory: Support,
    pub memory_misses: Support,
    pub memory_review: Support,
    pub memory_brief: Support,
    pub mcp_client_name: &'static str,
}

const FULL: ProviderSupport = ProviderSupport {
    session_started: Support::Supported,
    session_prompted: Support::Supported,
    session_tool: Support::Supported,
    session_ended: Support::Supported,
    cleanup_command: Support::Supported,
    plan_memory_step: Support::Supported,
    memory_mcp: Support::Supported,
    planning_memory: Support::Supported,
    memory_misses: Support::Supported,
    memory_review: Support::Supported,
    memory_brief: Support::Supported,
    mcp_client_name: "claude-code",
};

/// No wildcard arm: a new `Provider` variant fails to compile here.
#[must_use]
pub const fn provider_support(provider: Provider) -> ProviderSupport {
    match provider {
        Provider::ClaudeCode | Provider::T3 => FULL,
        Provider::Omp => ProviderSupport {
            mcp_client_name: "omp-coding-agent",
            ..FULL
        },
        Provider::OpenCode => ProviderSupport {
            session_ended: Support::NotSupported(
                "OpenCode has no session-end event; session.idle only ends a turn",
            ),
            memory_brief: Support::NotSupported(
                "OpenCode plugins cannot inject context at session start; the MCP instructions ask for the lists instead",
            ),
            mcp_client_name: "opencode",
            ..FULL
        },
    }
}

/// T3 hosts Claude Code and sends its name, so `claude-code` resolves to
/// `ClaudeCode`; a session's own state tells the two apart.
#[must_use]
pub fn provider_for_client_name(name: &str) -> Option<Provider> {
    Provider::SELECTABLE
        .into_iter()
        .find(|provider| provider_support(*provider).mcp_client_name == name)
}

#[cfg(test)]
#[path = "../../tests/unit/domain/capability.rs"]
mod tests;
