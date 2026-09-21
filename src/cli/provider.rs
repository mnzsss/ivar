use clap::{Args, Subcommand};

use crate::action::provider::add as provider_add;

/// The `ivar provider` surface: which harnesses a hall knows about.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List the hall's providers and the default one.
    List,
    /// Register a new provider by name.
    Add(ProviderAddArgs),
}

/// Arguments for `ivar provider add`.
#[derive(Debug, Args)]
pub struct ProviderAddArgs {
    /// The provider's name (e.g. `claude-code`, `opencode`, `omp`).
    pub name: String,
}

impl From<ProviderAddArgs> for provider_add::AddInput {
    fn from(args: ProviderAddArgs) -> Self {
        let ProviderAddArgs { name } = args;
        Self { name }
    }
}
