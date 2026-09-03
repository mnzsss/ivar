//! Closed facade dispatching provider-native behaviors by `Provider`.

pub mod claude_code;
pub mod omp;
pub mod opencode;

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::proc::Command;

/// What a provider harness can and cannot do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub supports_resume: bool,
    pub supports_review: bool,
    pub interactive: bool,
}

/// The launch specification for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchContract {
    pub binary: &'static str,
    pub capabilities: Capabilities,
}

/// Returns the launch contract (binary and capabilities) for a provider.
#[must_use]
pub fn launch_contract(provider: Provider) -> LaunchContract {
    match provider {
        Provider::ClaudeCode => claude_code::launch::contract(),
        Provider::OpenCode => opencode::launch::contract(),
        Provider::Omp => omp::launch::contract(),
    }
}

/// Builds the start command for a provider, validating resume capability.
pub fn start_command(provider: Provider, resume: bool) -> Result<Command, Failure> {
    let contract = launch_contract(provider);
    if resume && !contract.capabilities.supports_resume {
        return Err(Failure::blocked(
            "harness.no_resume",
            format!("`{}` cannot resume a session", contract.binary),
        )
        .expected("a harness whose capabilities include resume")
        .actual("this harness's `supports_resume` is false")
        .fix(FixAction::safe(
            "session.start_fresh",
            "Start a fresh session instead of resuming.",
        )));
    }
    match provider {
        Provider::ClaudeCode => Ok(claude_code::launch::start_command(resume)),
        Provider::OpenCode => Ok(opencode::launch::start_command(resume)),
        Provider::Omp => Ok(omp::launch::start_command(resume)),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/providers/mod.rs"]
mod tests;
