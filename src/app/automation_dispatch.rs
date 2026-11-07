use std::io;
use std::process::ExitCode;

use super::respond::respond;
use crate::action::Ctx;
use crate::action::automation::{decision, scan};
use crate::cli::automation::AutomationCommand;

pub(super) fn dispatch_automation(
    cmd: AutomationCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        AutomationCommand::Scan(args) => {
            respond(scan::scan(ctx, &args.into()), json, stdout, stderr)
        }
        AutomationCommand::Decide(args) => {
            respond(decision::decide(ctx, &args.into()), json, stdout, stderr)
        }
        AutomationCommand::Renew(args) => {
            respond(decision::renew(ctx, args.decision_id), json, stdout, stderr)
        }
        AutomationCommand::Decisions => respond(decision::decisions(ctx), json, stdout, stderr),
        AutomationCommand::Undo(args) => {
            respond(decision::undo(ctx, args.decision_id), json, stdout, stderr)
        }
    }
}
