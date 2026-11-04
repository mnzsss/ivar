//! CLI dispatch: turns a parsed [`Cli`] into an exit code by matching
//! `cli.command`, converting each variant's args into its action's `Input`,
//! and rendering the outcome. Dispatch lives here so the binary stays a
//! thin `main` — see ARCHITECTURE.md's module map.

use std::io::{self, Read as _, Write};
use std::process::ExitCode;

use clap::{CommandFactory as _, FromArgMatches as _};

use crate::action::Ctx;
use crate::action::confirm;
use crate::action::feature::select::{feature_from_env, resolve_single_feature};
use crate::action::hall;
use crate::action::mcp::add as mcp_add;
use crate::action::mcp::auth as mcp_auth;
use crate::action::mcp::edit as mcp_edit;
use crate::action::mcp::remove as mcp_remove;
use crate::action::mcp::status as mcp_status;
use crate::action::memory::capture::CaptureEnv;
use crate::action::provider::{
    add as provider_add, default as provider_default, list as provider_list,
    remove as provider_remove,
};
use crate::action::repo::{
    add, create as repo_create, list as repo_list, pull, remove, setup as repo_setup,
    upstream as repo_upstream, view as repo_view,
};
use crate::action::review::comment as review_comment;
use crate::action::session::guard_cmd as session_guard_cmd;
use crate::action::sync;
use crate::action::upgrade::Notice;
use crate::action::upgrade::command as upgrade_cmd;
use crate::app::respond::{current_dir, respond, respond_failure};
use crate::cli::MemoryCommand;
use crate::cli::root::{
    Cli, Command, CommentCommand, McpCommand, ProviderCommand, RepoCommand, ReviewCommand,
};
use crate::domain::provider::Provider;
use crate::infra::progress;
use crate::infra::term;

#[expect(
    clippy::too_many_lines,
    reason = "flat match over Command subcommands — each arm parses CLI args \
              into one *Input and delegates to one action fn's public entry \
              point; splitting the match into ad hoc helpers would only \
              relocate, not reduce, the branching"
)]
pub fn run(cli: Cli) -> ExitCode {
    if let Command::Memory(MemoryCommand::Capture { provider }) = &cli.command {
        capture_event(provider.as_deref());
        return ExitCode::SUCCESS;
    }
    let json = cli.json;
    let compact = cli.compact;
    // Prime both per-stream colour decisions with the flag, before any output
    // exists to render. `term`'s caches take their value from the first call
    // and ignore the argument afterwards, which is what lets `respond` and
    // `respond_failure` ask for the answer without every one of the ~60
    // dispatch arms below having to carry a palette down to them.
    //
    // Both streams are primed because they are redirected independently: the
    // value goes to stdout, failures and warnings to stderr.
    //
    // Hook verbs are read by a program, so they are primed off: `FORCE_COLOR`
    // or `--color always` in the user's shell must not reach a hook's bytes.
    let colour_override = if super::notice::is_hook_verb(&cli.command) {
        Some(false)
    } else {
        cli.color.as_override()
    };
    let _ = term::colour_for(term::Stream::Stdout, colour_override);
    let _ = term::colour_for(term::Stream::Stderr, colour_override);
    // indicatif paints through `console`, which has its own
    // global switch; give it ivar's stderr decision so it follows the flag.
    console::set_colors_enabled_stderr(term::colour_for(term::Stream::Stderr, colour_override));

    if !json && term::is_tty(term::Stream::Stdout) {
        term::prime_table_width(Some(term::width()));
    } else {
        term::prime_table_width(None);
    }

    // The progress sink, decided once for the same reason the colour caches are
    // primed above: `--json` is a machine-shaped run and wants no redraw line
    // even on stderr. `progress::reporter` asks the is-it-a-tty half.
    //
    // The confirmation seam is decided by the same rule: a `--json` run, a
    // `$CI` run, or a run with nobody on stderr may not prompt — a pipe is not
    // consent. Everything that must ask before it acts (`cleanup`, `migrate`,
    // and later integration's parent-promotion prompt) reads this one decision.
    let mut stdout = term::auto(io::stdout(), term::Stream::Stdout);
    let mut stderr = term::auto(io::stderr(), term::Stream::Stderr);

    // `-C <dir>` moves the whole process before anything reads the working
    // directory (hall discovery, session lookup, relative paths), so every
    // verb inherits it without a per-verb change.
    if let Err(failure) = apply_chdir(&cli.chdir) {
        return respond_failure(&failure, json, &mut stdout, &mut stderr);
    }

    let ctx = Ctx::new(current_dir())
        .with_session_feature(feature_from_env())
        .with_progress(progress::reporter(!json))
        .with_confirm(confirm::reporter(
            !json
                && std::env::var_os("CI").is_none()
                && term::is_tty(term::Stream::Stderr)
                && term::is_tty(term::Stream::Stdin),
        ));

    let session_id = std::env::var("IVAR_SESSION_ID").ok();

    // `--compact` is global for parsing's sake, but only `ivar graph` renders
    // it. Anywhere else it would be silently ignored and the human table
    // printed, so refuse before running (or mutating) anything.
    if compact && !matches!(cli.command, Command::Graph(_)) {
        return respond_failure(&compact_unsupported(), json, &mut stdout, &mut stderr);
    }

    // Decided from the cache before the command runs; printed after it, so
    // the line never interleaves with the command's own output.
    let notice = Notice::start(&super::notice::notice_context(json, &cli.command));

    let code = match cli.command {
        Command::Init(args) => respond(
            hall::init(&ctx, &args.into()),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Sync(args) => respond(
            sync::sync(&ctx, &args.into()),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Status => respond(hall::status(&ctx), json, &mut stdout, &mut stderr),
        Command::Doctor => respond(hall::doctor(&ctx), json, &mut stdout, &mut stderr),
        Command::Cleanup => respond(hall::cleanup(&ctx), json, &mut stdout, &mut stderr),
        Command::Migrate => respond(hall::migrate(&ctx), json, &mut stdout, &mut stderr),
        Command::Repo(cmd) => match cmd {
            RepoCommand::List => respond(repo_list::list(&ctx), json, &mut stdout, &mut stderr),
            RepoCommand::Add(args) => {
                respond(add::add(&ctx, args.into()), json, &mut stdout, &mut stderr)
            }
            RepoCommand::Create(args) => respond(
                repo_create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Remove(args) => respond(
                remove::remove(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Pull(args) => respond(
                pull::pull(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Setup(args) => respond(
                repo_setup::setup(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::View(args) => respond(
                repo_view::view(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Upstream(args) => respond(
                repo_upstream::upstream(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Feature(cmd) => super::feature_dispatch::dispatch_feature(
            cmd,
            &ctx,
            json,
            session_id.as_deref(),
            &mut stdout,
            &mut stderr,
        ),
        Command::Session(cmd) => super::session_dispatch::dispatch_session(
            cmd,
            &ctx,
            json,
            session_id.as_deref(),
            &mut stdout,
            &mut stderr,
        ),
        Command::Provider(cmd) => match cmd {
            ProviderCommand::List => {
                respond(provider_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
            ProviderCommand::Add(args) => respond(
                provider_add::add(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            ProviderCommand::Remove(args) => respond(
                provider_remove::remove(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            ProviderCommand::Default(args) => respond(
                provider_default::set_default(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Discovery(cmd) => {
            super::planning_dispatch::dispatch_discovery(cmd, &ctx, json, &mut stdout, &mut stderr)
        }
        Command::Review(ReviewCommand::Comment(mut cmd)) => {
            let explicit = cmd.feature_mut().take();
            match resolve_single_feature(&ctx, explicit, "Select a feature to review") {
                Ok(feature) => {
                    *cmd.feature_mut() = Some(feature);
                    match cmd {
                        CommentCommand::Add(args) => respond(
                            review_comment::add(&ctx, args.into()),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        CommentCommand::List(args) => respond(
                            review_comment::list(&ctx, args.into()),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        CommentCommand::Resolve(args) => respond(
                            review_comment::resolve(&ctx, &args.into()),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                    }
                }
                Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
            }
        }
        Command::Plan(cmd) => {
            super::planning_dispatch::dispatch_plan(cmd, &ctx, json, &mut stdout, &mut stderr)
        }
        Command::Skill(cmd) => {
            super::skill_dispatch::dispatch_skill(cmd, &ctx, json, &mut stdout, &mut stderr)
        }
        Command::Feedback(cmd) => {
            super::skill_dispatch::dispatch_feedback(cmd, &ctx, json, &mut stdout, &mut stderr)
        }
        Command::Guard(args) => {
            let input = match session_guard_cmd::GuardInput::try_from(args) {
                Ok(input) => input,
                Err(failure) => {
                    return respond_failure(&failure, json, &mut stdout, &mut stderr);
                }
            };
            match session_guard_cmd::run(&input) {
                Ok(outcome) => {
                    if !outcome.body.is_empty() {
                        let _ = write!(stdout, "{}", outcome.body);
                    }
                    if outcome.exit_zero {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
            }
        }
        Command::Mcp(cmd) => match cmd {
            McpCommand::Add(args) => respond(
                mcp_add::add(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            McpCommand::Edit(args) => respond(
                mcp_edit::edit(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            McpCommand::Remove(args) => respond(
                mcp_remove::remove(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            McpCommand::Auth(args) => respond(
                mcp_auth::auth(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            McpCommand::Status(args) => respond(
                mcp_status::status(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Upgrade(args) => respond(
            upgrade_cmd::upgrade(&args.into()),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Agent(cmd) => {
            super::agent_dispatch::dispatch_agent(cmd, &ctx, json, &mut stdout, &mut stderr)
        }
        Command::Graph(cmd) => super::graph_dispatch::dispatch_graph(
            cmd,
            &ctx,
            json,
            compact,
            &mut stdout,
            &mut stderr,
        ),
        Command::Memory(cmd) => match cmd {
            MemoryCommand::Capture { .. } => ExitCode::SUCCESS,
            MemoryCommand::Status => respond(
                crate::action::memory::status::status(&ctx),
                json,
                &mut stdout,
                &mut stderr,
            ),
            MemoryCommand::Clean => respond(
                crate::action::memory::clean::clean(&ctx),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        // Git's credential protocol is raw on stdin/stdout — it must not pass
        // through `respond`, which would render a `Report` on top of it.
        Command::GitCredential(args) => {
            match crate::git::credential::run(args.operation.as_deref()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    let _ = writeln!(io::stderr().lock(), "ivar: git-credential: {e}");
                    ExitCode::from(2)
                }
            }
        }
    };
    notice.finish(&mut stderr);
    code
}

/// Apply each `-C <dir>` in order, like `git -C`: a relative one is resolved
/// against the directory the previous one left the process in.
fn apply_chdir(dirs: &[std::path::PathBuf]) -> Result<(), crate::error::Failure> {
    for dir in dirs {
        std::env::set_current_dir(dir).map_err(|err| {
            crate::error::Failure::blocked(
                "cli.chdir_failed",
                format!("cannot change directory to `{}`: {err}", dir.display()),
            )
            .expected("`-C` to name an existing directory")
            .actual(format!("`{}` could not be entered", dir.display()))
        })?;
    }
    Ok(())
}

/// The refusal for `--compact` on a verb that has no compact form.
fn compact_unsupported() -> crate::error::Failure {
    crate::error::Failure::blocked(
        "cli.compact_unsupported",
        "`--compact` is only supported by `ivar graph`",
    )
    .expected("`--compact` together with an `ivar graph` verb")
    .actual("`--compact` on a verb outside `ivar graph`")
}

/// A payload past the cap is cut mid-document, fails to parse and is skipped,
/// so a huge tool input can never stall the agent's hook.
const MAX_CAPTURE_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024;

fn capture_event(provider: Option<&str>) {
    if let Some(provider) = provider.and_then(|id| id.parse::<Provider>().ok()) {
        crate::action::memory::capture::capture(
            provider,
            &read_capture_payload(),
            &CaptureEnv::from_process(),
        );
    }
}

fn is_capture_invocation(args: &[std::ffi::OsString]) -> bool {
    matches!(args, [_, memory, capture, ..] if memory == "memory" && capture == "capture")
}

fn lenient_provider(args: &[std::ffi::OsString]) -> Option<&str> {
    let mut args = args.iter().filter_map(|arg| arg.to_str());
    while let Some(arg) = args.next() {
        if arg == "--provider" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--provider=") {
            return Some(value);
        }
    }
    None
}

fn read_capture_payload() -> String {
    let mut bytes = Vec::new();
    let _ = io::stdin()
        .take(MAX_CAPTURE_PAYLOAD_BYTES)
        .read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Parse argv with the styled command. Pure: no stream, no exit.
pub(crate) fn try_parse(args: &[std::ffi::OsString]) -> Result<Cli, clap::Error> {
    let matches = Cli::command().try_get_matches_from(args)?;
    Cli::from_arg_matches(&matches)
}

/// Parse argv, or render clap's help/version/usage error and exit.
///
/// `ivar memory capture` is a hook that must never fail the agent, so a usage
/// error there exits 0 after capturing for whatever `--provider` was given.
///
/// Clap would decide colour itself (it ignores `FORCE_COLOR` and `--color`),
/// so the error is rendered here instead: on the stream clap names
/// (`use_stderr`: usage errors → stderr, `--help`/`--version` → stdout),
/// painted only if `term` says that stream may be. Priming `colour_for` with
/// the scanned flag is the same value `run` primes with afterwards.
#[allow(clippy::needless_pass_by_value)]
pub fn parse(args: Vec<std::ffi::OsString>) -> Cli {
    match try_parse(&args) {
        Ok(cli) => cli,
        Err(err) if err.use_stderr() && is_capture_invocation(&args) => {
            capture_event(lenient_provider(&args));
            std::process::exit(0);
        }
        Err(err) => {
            let stream = if err.use_stderr() {
                term::Stream::Stderr
            } else {
                term::Stream::Stdout
            };
            let colour = term::colour_for(stream, term::color_flag(&args));
            let rendered = err.render();
            let text = if colour {
                rendered.ansi().to_string()
            } else {
                rendered.to_string()
            };
            let _ = if err.use_stderr() {
                write!(io::stderr(), "{text}")
            } else {
                write!(io::stdout(), "{text}")
            };
            std::process::exit(err.exit_code());
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/run.rs"]
mod tests;
