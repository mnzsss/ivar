use clap::{Args, Subcommand};

use crate::action::mcp::add as mcp_add;
use crate::action::mcp::auth as mcp_auth;
use crate::action::mcp::edit as mcp_edit;
use crate::action::mcp::remove as mcp_remove;
use crate::action::mcp::status as mcp_status;

/// The `ivar mcp` surface: authenticating and inspecting the hall's declared MCP servers
/// under the session's provider.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Declare an MCP server in `ivar.json` — HTTP (`--url`, or a known name:
    /// `figma`, `linear`) or stdio (`--command`, its arguments after `--`) — and
    /// write it into every provider's MCP config. Adding a server already
    /// declared the same way changes nothing; the same name with a different
    /// definition is refused rather than overwritten. Prints the command to run
    /// next.
    Add(McpAddArgs),
    /// Replace a declared MCP server's definition — its HTTP URL or its stdio
    /// command — and rewrite every provider's MCP config. Changing the URL or
    /// the transport drops the server's OAuth registration, so run
    /// `ivar mcp auth` again; an identical definition changes nothing.
    Edit(McpEditArgs),
    /// Remove a declared MCP server from `ivar.json` and from every provider's
    /// MCP config, and delete its client secret from `.ivar/secrets/mcp.env`
    /// unless another server still uses it. Tokens a provider stored in its own
    /// credential store are left there.
    Remove(McpRemoveArgs),
    /// Authenticate one MCP server. Resolves the server from `ivar.json`'s
    /// `mcp` array and the provider from the hall's default, `--provider`, or
    /// — with `--all-providers` — every provider the hall lists, run one at a
    /// time; where a provider's own dynamic client registration is known to
    /// be rejected by the server (Figma on OpenCode / OMP, today), pre-registers a
    /// client first for that provider — a registration is not an
    /// authentication, and is reported separately, per provider. Persists the
    /// OAuth client secret locally into `.ivar/secrets/mcp.env` and endpoint metadata
    /// into `ivar.json` so subsequent sessions and authentications do not require
    /// re-exporting. For OMP, runs ivar's internal OAuth flow, installs tokens via
    /// auth-broker, and renders the `auth` block in `.omp/mcp.json` so OMP can
    /// natively refresh credentials. Then hands off to each provider's login path
    /// (`claude mcp login <name>`, `opencode mcp auth <name>`, or ivar's internal flow),
    /// which prints a URL and waits on a browser. With `--all-providers`, every
    /// provider is attempted even after an earlier one fails, and the command
    /// reports which succeeded and which failed rather than stopping at the first problem.
    Auth(McpAuthArgs),
    /// Report credential status across declared MCP servers and available providers.
    Status(McpStatusArgs),
}
/// Arguments for `ivar mcp add`.
#[derive(Debug, Args)]
pub struct McpAddArgs {
    /// The server's name in `ivar.json`'s `mcp` array. A known name (`figma`,
    /// `linear`) fills in the URL.
    pub name: String,
    /// The server's HTTP URL. Required for names ivar does not know, unless
    /// `--command` declares a stdio server instead.
    #[arg(long, conflicts_with = "command")]
    pub url: Option<String>,
    /// Declare a stdio (`local`) server spawned with this executable.
    #[arg(long)]
    pub command: Option<String>,
    /// Arguments for `--command`, after `--`.
    #[arg(last = true, allow_hyphen_values = true, requires = "command")]
    pub args: Vec<String>,
}

impl From<McpAddArgs> for mcp_add::AddInput {
    fn from(args: McpAddArgs) -> Self {
        let McpAddArgs {
            name,
            url,
            command,
            args,
        } = args;
        Self {
            name,
            url,
            command,
            args,
        }
    }
}

/// Arguments for `ivar mcp edit`.
#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("definition").required(true).args(["url", "command"])))]
pub struct McpEditArgs {
    /// The server's name, as declared in `ivar.json`'s `mcp` array.
    pub name: String,
    /// The server's new HTTP URL.
    #[arg(long)]
    pub url: Option<String>,
    /// Make it a stdio (`local`) server spawned with this executable.
    #[arg(long)]
    pub command: Option<String>,
    /// Arguments for `--command`, after `--`.
    #[arg(last = true, allow_hyphen_values = true, requires = "command")]
    pub args: Vec<String>,
}

impl From<McpEditArgs> for mcp_edit::EditInput {
    fn from(args: McpEditArgs) -> Self {
        let McpEditArgs {
            name,
            url,
            command,
            args,
        } = args;
        Self {
            name,
            url,
            command,
            args,
        }
    }
}

/// Arguments for `ivar mcp remove`.
#[derive(Debug, Args)]
pub struct McpRemoveArgs {
    /// The server's name, as declared in `ivar.json`'s `mcp` array.
    pub name: String,
}

impl From<McpRemoveArgs> for mcp_remove::RemoveInput {
    fn from(args: McpRemoveArgs) -> Self {
        let McpRemoveArgs { name } = args;
        Self { name }
    }
}

/// Arguments for `ivar mcp auth`.
#[derive(Debug, Args)]
pub struct McpAuthArgs {
    /// The server's name, as declared in `ivar.json`'s `mcp` array.
    pub server: String,
    /// The provider to authenticate against. Defaults to the hall's default
    /// provider. Conflicts with `--all-providers`.
    #[arg(long, conflicts_with = "all_providers")]
    pub provider: Option<String>,
    /// Authenticate every provider the hall lists (`providers.available`),
    /// one at a time — never concurrently, since each provider's login
    /// command takes over the terminal and waits on a browser. Every
    /// provider is attempted even if an earlier one fails; the run is
    /// reported as needing attention (not a clean success) the moment any of
    /// them does. Conflicts with `--provider`.
    #[arg(long)]
    pub all_providers: bool,
}

impl From<McpAuthArgs> for mcp_auth::AuthInput {
    fn from(args: McpAuthArgs) -> Self {
        let McpAuthArgs {
            server,
            provider,
            all_providers,
        } = args;
        Self {
            server,
            provider,
            all_providers,
        }
    }
}

/// Arguments for `ivar mcp status`.
#[derive(Debug, Args)]
pub struct McpStatusArgs {
    /// The server's name, as declared in `ivar.json`'s `mcp` array. When omitted, reports all servers.
    pub server: Option<String>,
    /// The provider to inspect. Defaults to checking all available providers in the hall.
    #[arg(long)]
    pub provider: Option<String>,
    /// Query live harness status via provider CLI rather than inspecting local credential stores only.
    #[arg(long)]
    pub live: bool,
}

impl From<McpStatusArgs> for mcp_status::StatusInput {
    fn from(args: McpStatusArgs) -> Self {
        let McpStatusArgs {
            server,
            provider,
            live,
        } = args;
        Self {
            server,
            provider,
            live,
        }
    }
}
