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
pub struct ProviderSupport {}

/// No wildcard arm: a new `Provider` variant fails to compile here.
#[must_use]
pub const fn provider_support(provider: Provider) -> ProviderSupport {
    match provider {
        Provider::ClaudeCode => ProviderSupport {},
        Provider::OpenCode => ProviderSupport {},
        Provider::Omp => ProviderSupport {},
        Provider::T3 => ProviderSupport {},
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/capability.rs"]
mod tests;
