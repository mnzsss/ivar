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

use std::io;
use std::process::ExitCode;

use camino::Utf8PathBuf;
use clap::Parser;
use serde::Serialize;

use ivar::action::Ctx;
use ivar::action::feature::{create, demote, list as feature_list, promote, status};
use ivar::action::hall::{self, InitInput};
use ivar::action::repo::{add, list as repo_list, pull, remove};
use ivar::action::session::start as session_start;
use ivar::action::sync::{self, SyncInput};
use ivar::cli::root::{Cli, Command, FeatureCommand, RepoCommand, SessionCommand};
use ivar::error::{Failure, Outcome, Report, WriteHuman};
use ivar::infra::term;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;

    // Decided once, from the flag plus the real environment/tty — no
    // renderer in this slice applies colour yet (`Failure::write_human`
    // deliberately does not), but the flag already feeds the one function
    // that will drive every future one.
    let _colour = term::colour(cli.color.as_override());

    let ctx = Ctx::new(current_dir());

    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    match cli.command {
        Command::Init(args) => respond(
            hall::init(&ctx, InitInput::from(args)),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Sync(args) => respond(
            sync::sync(&ctx, SyncInput::from(args)),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Status => {
            respond_failure(not_implemented("status"), json, &mut stdout, &mut stderr)
        }
        Command::Doctor => {
            respond_failure(not_implemented("doctor"), json, &mut stdout, &mut stderr)
        }
        Command::Cleanup => {
            respond_failure(not_implemented("cleanup"), json, &mut stdout, &mut stderr)
        }
        Command::Repo(cmd) => match cmd {
            RepoCommand::List => respond(repo_list::list(&ctx), json, &mut stdout, &mut stderr),
            RepoCommand::Add(args) => respond(
                add::add(
                    &ctx,
                    add::AddInput {
                        name: args.name,
                        url: args.url,
                        default_branch: args.default_branch,
                        reuse_existing: if args.fresh {
                            Some(false)
                        } else if args.reuse {
                            Some(true)
                        } else {
                            None
                        },
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Remove(args) => respond(
                remove::remove(&ctx, remove::RemoveInput { name: args.name }),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Pull(args) => respond(
                pull::pull(&ctx, pull::PullInput { repo: args.repo }),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Feature(cmd) => match cmd {
            FeatureCommand::Create(args) => respond(
                create::create(&ctx, create::CreateInput { name: args.name }),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::List => {
                respond(feature_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
            FeatureCommand::Promote(args) => respond(
                promote::promote(
                    &ctx,
                    promote::PromoteInput {
                        feature: args.feature,
                        repo: args.repo,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Demote(args) => respond(
                demote::demote(
                    &ctx,
                    demote::DemoteInput {
                        feature: args.feature,
                        repo: args.repo,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Status(args) => respond(
                status::status(&ctx, status::StatusInput { feature: args.feature }),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Session(cmd) => match cmd {
            SessionCommand::Start(args) => respond(
                session_start::start(
                    &ctx,
                    session_start::StartInput {
                        feature: args.feature,
                        resume: args.resume,
                        provider: args.provider,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Provider => {
            respond_failure(not_implemented("provider"), json, &mut stdout, &mut stderr)
        }
        Command::Plan => respond_failure(not_implemented("plan"), json, &mut stdout, &mut stderr),
        Command::Skill => respond_failure(not_implemented("skill"), json, &mut stdout, &mut stderr),
    }
}

/// A root verb that exists in the settled surface (ARCHITECTURE.md names all
/// eleven) but has not landed yet. Never a silent success and never
/// `todo!()` (that lint is denied crate-wide) — a [`Failure`] that names the
/// verb, so `--json` and the human surface agree on exactly what is missing.
fn not_implemented(verb: &str) -> Failure {
    Failure::blocked(
        "cli.not_implemented",
        format!("`ivar {verb}` is not implemented yet"),
    )
    .expected("a verb that has shipped")
    .actual("this verb is named in the root surface but not wired up in this slice")
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

/// Render `failure`, `--json` or human, and always exit `2`.
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
                let _ = report.value.write_human(stdout);
                for warning in &report.warnings {
                    let _ = writeln!(stderr, "{warning}");
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
        let _ = failure.write_human(stderr);
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
