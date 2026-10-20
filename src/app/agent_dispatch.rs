use std::io;
use std::process::ExitCode;

use super::respond::respond;
use crate::action::{Ctx, agent};
use crate::cli::agent::AgentCommand;

pub(super) fn dispatch_agent(
    cmd: AgentCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        AgentCommand::Sync => respond(agent::sync::sync(ctx), json, stdout, stderr),
        AgentCommand::Status => respond(agent::status::status(ctx), json, stdout, stderr),
        AgentCommand::Run(args) => respond(
            agent::run::RunInput::from_cli(args.name, &args.trigger)
                .and_then(|input| agent::run::run(ctx, input)),
            json,
            stdout,
            stderr,
        ),
        AgentCommand::Logs(args) => {
            respond(agent::logs::logs(ctx, args.into()), json, stdout, stderr)
        }
        AgentCommand::Feedback(args) => respond(
            agent::feedback::feedback(ctx, args.into()),
            json,
            stdout,
            stderr,
        ),
    }
}
