<!-- Generated from clap by tests/static_checks/docs_reference.rs. Do not edit by hand: run
     `IVAR_UPDATE_DOCS=1 cargo test --test static_checks`. -->

# `ivar mcp`

Authenticate the hall's declared MCP servers


## `ivar mcp add`

Declare an MCP server in `ivar.json` — HTTP (`--url`, or a known name: `figma`, `linear`, `slack`) or stdio (`--command`, its arguments after `--`) — and write it into every provider's MCP config. Adding a server already declared the same way changes nothing; the same name with a different definition is refused rather than overwritten. Prints the command to run next

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The server's name in `ivar.json`'s `mcp` array. A known name (`figma`, `linear`, `slack`) fills in the URL |
| `args` | no | Arguments for `--command`, after `--` |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--url` | `<URL>` |  | The server's HTTP URL. Required for names ivar does not know, unless `--command` declares a stdio server instead |
| `--command` | `<COMMAND>` |  | Declare a stdio (`local`) server spawned with this executable |


## `ivar mcp edit`

Replace a declared MCP server's definition — its HTTP URL or its stdio command — and rewrite every provider's MCP config. Changing the URL or the transport drops the server's OAuth registration, so run `ivar mcp auth` again; an identical definition changes nothing

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The server's name, as declared in `ivar.json`'s `mcp` array |
| `args` | no | Arguments for `--command`, after `--` |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--url` | `<URL>` |  | The server's new HTTP URL |
| `--command` | `<COMMAND>` |  | Make it a stdio (`local`) server spawned with this executable |


## `ivar mcp remove`

Remove a declared MCP server from `ivar.json` and from every provider's MCP config, and delete its client secret from `.ivar/secrets/mcp.env` unless another server still uses it. Tokens a provider stored in its own credential store are left there

| argument | required | description |
| --- | --- | --- |
| `name` | yes | The server's name, as declared in `ivar.json`'s `mcp` array |


## `ivar mcp auth`

Authenticate one MCP server. Resolves the server from `ivar.json`'s `mcp` array and the provider from the hall's default, `--provider`, or — with `--all-providers` — every provider the hall lists, run one at a time; where a provider's own dynamic client registration is known to be rejected by the server (Figma on OpenCode / OMP, today), pre-registers a client first for that provider — a registration is not an authentication, and is reported separately, per provider. Persists the OAuth client secret locally into `.ivar/secrets/mcp.env` and endpoint metadata into `ivar.json` so subsequent sessions and authentications do not require re-exporting. For OMP, runs ivar's internal OAuth flow, installs tokens via auth-broker, and renders the `auth` block in `.omp/mcp.json` so OMP can natively refresh credentials. Then hands off to each provider's login path (`claude mcp login <name>`, `opencode mcp auth <name>`, or ivar's internal flow), which prints a URL and waits on a browser. With `--all-providers`, every provider is attempted even after an earlier one fails, and the command reports which succeeded and which failed rather than stopping at the first problem

| argument | required | description |
| --- | --- | --- |
| `server` | yes | The server's name, as declared in `ivar.json`'s `mcp` array |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | The provider to authenticate against. Defaults to the hall's default provider. Conflicts with `--all-providers` |
| `--all-providers` |  |  | Authenticate every provider the hall lists (`providers.available`), one at a time — never concurrently, since each provider's login command takes over the terminal and waits on a browser. Every provider is attempted even if an earlier one fails; the run is reported as needing attention (not a clean success) the moment any of them does. Conflicts with `--provider` |


## `ivar mcp status`

Report credential status across declared MCP servers and available providers

| argument | required | description |
| --- | --- | --- |
| `server` | no | The server's name, as declared in `ivar.json`'s `mcp` array. When omitted, reports all servers |

| flag | value | default | description |
| --- | --- | --- | --- |
| `--provider` | `<PROVIDER>` |  | The provider to inspect. Defaults to checking all available providers in the hall |
| `--live` |  |  | Query live harness status via provider CLI rather than inspecting local credential stores only |

