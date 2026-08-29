//! Entrypoint. Parses argv, dispatches into `action`, renders the outcome,
//! sets the exit code. No logic beyond that plumbing lives here — see
//! ARCHITECTURE.md's module map.
//!
//! # Exit codes
//!
//! - `0` — a clean [`Report`]: the value, no warnings.
//! - `1` — a `Report` carrying warnings: the operation went through but
//!   something needs attention. [`Report::is_clean`] is the switch.
//! - `2` — a [`Failure`]: refused before starting, or broke mid-flight.
//!
//! `--json` prints exactly the `Report` or `Failure` value the action
//! returned — never a second, hand-formatted summary — so the machine
//! surface and the human text below it can never drift apart
//! (ARCHITECTURE.md, "1. `action` is the unit, and it has one output
//! shape").

use std::io::{self, Write};
use std::process::ExitCode;

use camino::Utf8PathBuf;
use clap::Parser;
use serde::Serialize;

use ivar::action::Ctx;
use ivar::action::confirm;
use ivar::action::execute::{accept_revision, finish, start, status as execute_status};
use ivar::action::feature::{
    cleanup, close, create, delete, deliver, demote, integrate, list as feature_list, promote,
    prune as feature_prune, rebase, rename, reparent, review, status, view,
};
use ivar::action::hall;
use ivar::action::mcp::auth as mcp_auth;
use ivar::action::plan::approve::{self as plan_approve};
use ivar::action::plan::{
    create as plan_create, list as plan_list, show as plan_show, status as plan_status,
};
use ivar::action::provider::{add as provider_add, list as provider_list};
use ivar::action::repo::{
    add, list as repo_list, pull, remove, setup as repo_setup, upstream as repo_upstream,
};
use ivar::action::session::{
    connect as session_connect, conversion as session_conversion, prune as session_prune,
    relay as session_relay, start as session_start, stop as session_stop,
};
use ivar::action::skill::{
    add as skill_add, create as skill_create, detach as skill_detach, doctor as skill_doctor,
    list as skill_list, remove as skill_remove, status as skill_status, sync as skill_sync,
    update as skill_update,
};
use ivar::action::sync;
use ivar::cli::root::{
    Cli, Command, ExecuteCommand, FeatureCommand, McpCommand, PlanCommand, ProviderCommand,
    RepoCommand, SessionCommand, SkillCommand,
};
use ivar::error::{Failure, Outcome, Palette, Report, WriteHuman};
use ivar::infra::progress;
use ivar::infra::term;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;

    // Prime both per-stream colour decisions with the flag, before any output
    // exists to render. `term`'s caches take their value from the first call
    // and ignore the argument afterwards, which is what lets `respond` and
    // `respond_failure` ask for the answer without every one of the ~60
    // dispatch arms below having to carry a palette down to them.
    //
    // Both streams are primed because they are redirected independently: the
    // value goes to stdout, failures and warnings to stderr.
    let _ = term::colour_for(term::Stream::Stdout, cli.color.as_override());
    let _ = term::colour_for(term::Stream::Stderr, cli.color.as_override());

    // The progress sink, decided once for the same reason the colour caches are
    // primed above: `--json` is a machine-shaped run and wants no redraw line
    // even on stderr. `progress::reporter` asks the is-it-a-tty half.
    //
    // The confirmation seam is decided by the same rule: a `--json` run, a
    // `$CI` run, or a run with nobody on stderr may not prompt — a pipe is not
    // consent. Everything that must ask before it acts (`cleanup`, `migrate`,
    // and later integration's parent-promotion prompt) reads this one decision.
    let ctx = Ctx::new(current_dir())
        .with_progress(progress::reporter(!json))
        .with_confirm(confirm::reporter(
            !json
                && std::env::var_os("CI").is_none()
                && term::is_tty(term::Stream::Stderr)
                && term::is_tty(term::Stream::Stdin),
        ));

    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    match cli.command {
        Command::Init(args) => respond(
            hall::init(&ctx, args.into()),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Sync(args) => respond(
            sync::sync(&ctx, args.into()),
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
            RepoCommand::Remove(args) => respond(
                remove::remove(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Pull(args) => respond(
                pull::pull(&ctx, args.into()),
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
            RepoCommand::Upstream(args) => respond(
                repo_upstream::upstream(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Feature(cmd) => match cmd {
            FeatureCommand::Create(args) => respond(
                create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::List => {
                respond(feature_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
            FeatureCommand::Promote(args) => respond(
                promote::promote(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Demote(args) => respond(
                demote::demote(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Status(args) => respond(
                status::status(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Reparent(args) => respond(
                reparent::reparent(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Rename(args) => respond(
                rename::rename(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Integrate(args) => respond(
                integrate::integrate(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Execute(cmd) => match cmd {
                ExecuteCommand::Start(args) => respond(
                    start::start(&ctx, args.into()),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::Finish(args) => respond(
                    finish::finish(&ctx, args.into()),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::Status(args) => respond(
                    execute_status::status(&ctx, args.into()),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::AcceptRevision(args) => respond(
                    accept_revision::accept_revision(&ctx, args.into()),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
            },
            FeatureCommand::Deliver(args) => respond(
                deliver::deliver(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Close(args) => respond(
                close::close(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Delete(args) => respond(
                delete::delete(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Cleanup(args) => respond(
                cleanup::cleanup(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Rebase(args) => respond(
                rebase::rebase(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Review(args) => respond(
                review::review(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::View(args) => respond(
                view::view(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Prune => {
                respond(feature_prune::prune(&ctx), json, &mut stdout, &mut stderr)
            }
        },
        Command::Session(cmd) => match cmd {
            SessionCommand::Start(args) => respond(
                session_start::start(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Connect(args) => respond(
                session_connect::connect(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Convert(args) => respond(
                session_conversion::convert(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Stop(args) => respond(
                session_stop::stop(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Prune => {
                respond(session_prune::prune(&ctx), json, &mut stdout, &mut stderr)
            }
            SessionCommand::Relay(args) => respond(
                session_relay::relay(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Provider(cmd) => match cmd {
            ProviderCommand::List => {
                respond(provider_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
            ProviderCommand::Add(args) => respond(
                provider_add::add(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Plan(cmd) => match cmd {
            PlanCommand::Create(args) => respond(
                plan_create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::List => respond(plan_list::list(&ctx), json, &mut stdout, &mut stderr),
            PlanCommand::Show(args) => respond(
                plan_show::show(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::Approve(args) => respond(
                plan_approve::approve(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::Invalidate(args) => respond(
                plan_approve::invalidate(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::Status(args) => respond(
                plan_status::status(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Skill(cmd) => match cmd {
            SkillCommand::List => respond(skill_list::list(&ctx), json, &mut stdout, &mut stderr),
            SkillCommand::Create(args) => respond(
                skill_create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Add(args) => respond(
                skill_add::add(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Update(args) => respond(
                skill_update::update(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Remove(args) => respond(
                skill_remove::remove(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Detach(args) => respond(
                skill_detach::detach(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Sync => respond(skill_sync::sync(&ctx), json, &mut stdout, &mut stderr),
            SkillCommand::Status => {
                respond(skill_status::status(&ctx), json, &mut stdout, &mut stderr)
            }
            SkillCommand::Doctor => {
                respond(skill_doctor::doctor(&ctx), json, &mut stdout, &mut stderr)
            }
        },
        Command::Mcp(cmd) => match cmd {
            McpCommand::Auth(args) => respond(
                mcp_auth::auth(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        // Git's credential protocol is raw on stdin/stdout — it must not pass
        // through `respond`, which would render a `Report` on top of it.
        Command::GitCredential(args) => {
            match ivar::git::credential::run(args.operation.as_deref()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    let _ = writeln!(io::stderr().lock(), "ivar: git-credential: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}

/// Render a [`Report`]'s exit code: clean is `0`, warnings present is `1`.
/// [`Report::is_clean`] is the one switch — see the module doc comment.
fn exit_code_for<T>(report: &Report<T>) -> ExitCode {
    if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// The palette for whatever is being written to stderr — failures and warnings.
///
/// Reads the decision `main` already primed, so the flag is honoured without
/// being threaded through every dispatch arm.
fn stderr_palette() -> Palette {
    Palette::from_decision(term::colour_for(term::Stream::Stderr, None))
}

/// Render whatever an action returned, and pick the exit code.
///
/// The one place the success half of an [`Outcome`] is turned into bytes, so
/// every verb renders identically and neither `--json` nor the human text can
/// acquire a second, hand-written formatting path. `respond_failure` is the
/// error half of the same pair.
fn respond<T>(
    result: Outcome<T>,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode
where
    T: Serialize + WriteHuman,
{
    match result {
        Ok(report) => {
            let exit = exit_code_for(&report);
            if json {
                let _ = write_json(stdout, &report);
            } else {
                // The value is never painted: it is data, and the --json
                // surface shows the same strings raw.
                let _ = report.value.write_human(stdout);
                let palette = stderr_palette();
                for warning in &report.warnings {
                    let _ = warning.write_painted(stderr, &palette);
                }
            }
            exit
        }
        Err(failure) => respond_failure(failure, json, stdout, stderr),
    }
}

fn respond_failure(
    failure: Failure,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    if json {
        let _ = write_json(stdout, &failure);
    } else {
        let _ = failure.write_painted(stderr, &stderr_palette());
    }
    ExitCode::from(2)
}

/// The `--json` surface: the value's `Serialize` form, one line, to `w`.
fn write_json(w: &mut impl io::Write, value: &impl Serialize) -> io::Result<()> {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"status":"failed","code":"cli.render_failed","what":"could not render JSON output"}"#
            .to_owned()
    });
    writeln!(w, "{rendered}")
}

/// The real process's current directory, as a [`Utf8PathBuf`]. Falls back to
/// `.` if it cannot be read or is not valid UTF-8 — vanishingly rare, and
/// `Ctx::resolve` still does the right thing with a relative fallback.
fn current_dir() -> Utf8PathBuf {
    let raw = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    Utf8PathBuf::from_path_buf(raw).unwrap_or_else(|_| Utf8PathBuf::from("."))
}
