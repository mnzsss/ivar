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
use ivar::action::execute::{
    ack as execute_ack, prepare, reconcile as execute_reconcile, replan as execute_replan,
};
use ivar::action::feature::{
    close, create, delete, deliver, demote, list as feature_list, promote, rebase, review, status,
    view,
};
use ivar::action::hall::{self, InitInput};
use ivar::action::plan::approve::{self as plan_approve};
use ivar::action::plan::{create as plan_create, list as plan_list, show as plan_show};
use ivar::action::provider::list as provider_list;
use ivar::action::repo::{add, list as repo_list, pull, remove};
use ivar::action::session::{
    connect as session_connect, conversion as session_conversion, start as session_start,
};
use ivar::action::skill::{create as skill_create, list as skill_list};
use ivar::action::sync::{self, SyncInput};
use ivar::cli::root::{
    Cli, Command, ExecuteCommand, FeatureCommand, PlanCommand, ProviderCommand, RepoCommand,
    SessionCommand, SkillCommand,
};
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
        Command::Status => respond(hall::status(&ctx), json, &mut stdout, &mut stderr),
        Command::Doctor => respond(hall::doctor(&ctx), json, &mut stdout, &mut stderr),
        Command::Cleanup => respond(hall::cleanup(&ctx), json, &mut stdout, &mut stderr),
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
                remove::remove(
                    &ctx,
                    remove::RemoveInput {
                        name: args.name,
                        force: args.force,
                    },
                ),
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
                status::status(
                    &ctx,
                    status::StatusInput {
                        feature: args.feature,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Execute(cmd) => match cmd {
                ExecuteCommand::Prepare(args) => respond(
                    prepare::prepare(
                        &ctx,
                        prepare::PrepareInput {
                            feature: args.feature,
                            graph_json: args.graph_json,
                        },
                    ),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::Replan(args) => respond(
                    execute_replan::replan(
                        &ctx,
                        execute_replan::ReplanInput {
                            feature: args.feature,
                            plan: args.plan,
                        },
                    ),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::AckRevision(args) => respond(
                    execute_ack::ack_revision(
                        &ctx,
                        execute_ack::AckInput {
                            feature: args.feature,
                            workstream: args.workstream,
                        },
                    ),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::Reconcile(args) => respond(
                    execute_reconcile::reconcile(
                        &ctx,
                        execute_reconcile::ReconcileInput {
                            feature: args.feature,
                            workstream: args.workstream,
                            description: args.description,
                        },
                    ),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
            },
            FeatureCommand::Deliver(args) => respond(
                deliver::deliver(
                    &ctx,
                    deliver::DeliverInput {
                        feature: args.name,
                        preview: args.preview,
                        fingerprint: args.fingerprint,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Close(args) => respond(
                close::close(
                    &ctx,
                    close::CloseInput {
                        name: args.name,
                        outcome: args.outcome,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Delete(args) => respond(
                delete::delete(&ctx, delete::DeleteInput { name: args.name }),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Rebase(args) => respond(
                rebase::rebase(&ctx, rebase::RebaseInput { name: args.name }),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::Review(args) => respond(
                review::review(&ctx, review::ReviewInput { name: args.name }),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::View { name } => respond(
                view::view(&ctx, view::ViewInput { feature: name }),
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
                        detached: args.detached,
                        relay: args.relay,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Connect(args) => respond(
                session_connect::connect(
                    &ctx,
                    session_connect::ConnectInput {
                        session_id: args.session_id,
                        feature: args.feature,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Convert(args) => respond(
                session_conversion::convert(
                    &ctx,
                    session_conversion::ConvertInput {
                        session_id: args.session_id,
                        feature: args.feature,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Provider(cmd) => match cmd {
            ProviderCommand::List => {
                respond(provider_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
        },
        Command::Plan(cmd) => match cmd {
            PlanCommand::Create(args) => respond(
                plan_create::create(
                    &ctx,
                    plan_create::CreateInput {
                        feature: args.feature,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::List => respond(plan_list::list(&ctx), json, &mut stdout, &mut stderr),
            PlanCommand::Show(args) => respond(
                plan_show::show(
                    &ctx,
                    plan_show::ShowInput {
                        feature: args.feature,
                        artifact: args.artifact,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::Approve(args) => respond(
                plan_approve::approve(
                    &ctx,
                    plan_approve::ApproveInput {
                        feature: args.feature,
                        gate: args.gate,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
            PlanCommand::Invalidate(args) => respond(
                plan_approve::invalidate(
                    &ctx,
                    plan_approve::InvalidateInput {
                        feature: args.feature,
                        gate: args.gate,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Skill(cmd) => match cmd {
            SkillCommand::List => respond(skill_list::list(&ctx), json, &mut stdout, &mut stderr),
            SkillCommand::Create(args) => respond(
                skill_create::create(
                    &ctx,
                    skill_create::CreateInput {
                        id: args.id,
                        description: args.description,
                    },
                ),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
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
