use clap::{Args, Subcommand};

use crate::action::provider::add as provider_add;
use crate::action::provider::default as provider_default;
use crate::action::provider::remove as provider_remove;

/// The `ivar provider` surface: which harnesses a hall knows about.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List the hall's providers and the default one.
    List,
    /// Register a new provider by name and materialise its config and skills.
    Add(ProviderAddArgs),
    /// Unregister a provider and tear down its hall config and skills.
    Remove(ProviderRemoveArgs),
    /// Make a registered provider the hall's default — the one `ivar session
    /// start` and `ivar mcp auth` use when none is named. Sessions already
    /// running keep their provider.
    Default(ProviderDefaultArgs),
}

/// Arguments for `ivar provider add`.
#[derive(Debug, Args)]
pub struct ProviderAddArgs {
    /// The provider's name (e.g. `claude-code`, `opencode`, `omp`, `t3`).
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
    /// The provider's name (e.g. `claude-code`, `opencode`, `omp`, `t3`).
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

/// Arguments for `ivar provider default`.
#[derive(Debug, Args)]
pub struct ProviderDefaultArgs {
    /// The provider's name (e.g. `claude-code`, `opencode`, `omp`, `t3`).
    pub name: String,
}

impl From<ProviderDefaultArgs> for provider_default::DefaultInput {
    fn from(args: ProviderDefaultArgs) -> Self {
        let ProviderDefaultArgs { name } = args;
        Self { name }
    }
}
