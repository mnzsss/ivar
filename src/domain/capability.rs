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
}

const FULL: ProviderSupport = ProviderSupport {
    session_started: Support::Supported,
    session_prompted: Support::Supported,
    session_tool: Support::Supported,
    session_ended: Support::Supported,
    cleanup_command: Support::Supported,
    plan_memory_step: Support::Supported,
};

/// No wildcard arm: a new `Provider` variant fails to compile here.
#[must_use]
pub const fn provider_support(provider: Provider) -> ProviderSupport {
    match provider {
        Provider::ClaudeCode | Provider::Omp | Provider::T3 => FULL,
        Provider::OpenCode => ProviderSupport {
            session_ended: Support::NotSupported(
                "OpenCode has no session-end event; session.idle only ends a turn",
            ),
            ..FULL
        },
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/capability.rs"]
mod tests;
