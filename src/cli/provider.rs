use clap::{Args, Subcommand};

use crate::action::provider::add as provider_add;
use crate::action::provider::remove as provider_remove;

/// The `ivar provider` surface: which harnesses a hall knows about.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List the hall's providers and the default one.
    List,
    /// Register a new provider by name.
    Add(ProviderAddArgs),
    /// Unregister a provider and tear down its hall config.
    Remove(ProviderRemoveArgs),
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

/// Arguments for `ivar provider remove`.
#[derive(Debug, Args)]
pub struct ProviderRemoveArgs {
    /// The provider's name (e.g. `claude-code`, `opencode`, `omp`).
    pub name: String,
    /// The provider to make the default. Required when removing the current default
    #[arg(long)]
    pub default: Option<String>,
}

impl From<ProviderRemoveArgs> for provider_remove::RemoveInput {
    fn from(args: ProviderRemoveArgs) -> Self {
        let ProviderRemoveArgs { name, default } = args;
        Self { name, default }
    }
}
