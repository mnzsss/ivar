use std::io;
use std::process::ExitCode;

use super::respond::respond;
use crate::action::Ctx;
use crate::action::automation::scan;
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
    }
}
