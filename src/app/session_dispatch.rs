use std::io;
use std::process::ExitCode;

use super::respond::{respond, respond_failure};
use crate::action::Ctx;
use crate::action::feature::select::resolve_single_feature;
use crate::action::session::{
    connect as session_connect, conversion as session_conversion, env_cmd as session_env_cmd,
    list as session_list, prune as session_prune, relay as session_relay, start as session_start,
    stop as session_stop,
};
use crate::cli::root::SessionCommand;

pub(super) fn dispatch_session(
    cmd: SessionCommand,
    ctx: &Ctx,
    json: bool,
    session_id: Option<&str>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        SessionCommand::Start(args) => {
            respond(session_start::start(ctx, args.into()), json, stdout, stderr)
        }
        SessionCommand::List => respond(session_list::list(ctx), json, stdout, stderr),
        SessionCommand::Connect(args) => respond(
            session_connect::connect(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SessionCommand::Convert(args) => respond(
            session_conversion::convert(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SessionCommand::Stop(args) => {
            let mut input: session_stop::StopInput = args.into();
            input.session = input.session.or_else(|| session_id.map(str::to_owned));
            respond(session_stop::stop(ctx, &input), json, stdout, stderr)
        }
        SessionCommand::Prune => respond(session_prune::prune(ctx), json, stdout, stderr),
        SessionCommand::Relay(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to relay") {
                Ok(feature) => respond(
                    session_relay::relay(
                        ctx,
                        session_relay::RelayInput {
                            feature,
                            provider: args.provider,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        SessionCommand::Env(args) => respond(
            session_env_cmd::run(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SessionCommand::Sandbox(args) => {
            match crate::action::session::sandbox::run_launcher(
                ctx,
                &args.session,
                args.resume,
                args.prompt.as_deref(),
                &args.command,
            ) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
    }
}
