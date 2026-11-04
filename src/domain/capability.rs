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
}

const FULL: ProviderSupport = ProviderSupport {
    session_started: Support::Supported,
    session_prompted: Support::Supported,
    session_tool: Support::Supported,
    session_ended: Support::Supported,
};

/// No wildcard arm: a new `Provider` variant fails to compile here.
#[must_use]
pub const fn provider_support(provider: Provider) -> ProviderSupport {
    match provider {
        Provider::ClaudeCode | Provider::Omp => FULL,
        Provider::OpenCode => ProviderSupport {
            session_ended: Support::NotSupported(
                "OpenCode has no session-end event; session.idle only ends a turn",
            ),
            ..FULL
        },
        Provider::T3 => ProviderSupport {
            session_ended: Support::NotSupported(
                "SessionEnd on T3 is not confirmed; the matrix changes only after a recorded live run",
            ),
            ..FULL
        },
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/capability.rs"]
mod tests;
