use clap::{Args, Subcommand};

use crate::action::mcp::auth as mcp_auth;

/// The `ivar mcp` surface: authenticating the hall's declared MCP servers
/// under the session's provider.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
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
