use std::io;
use std::process::ExitCode;

use super::respond::respond;
use crate::action::Ctx;
use crate::action::memory::{clean, lint, query, reindex, review, status};
use crate::cli::MemoryCommand;

pub(super) fn dispatch_memory(
    cmd: MemoryCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        MemoryCommand::Capture { .. } => ExitCode::SUCCESS,
        MemoryCommand::Status => respond(status::status(ctx), json, stdout, stderr),
        MemoryCommand::Clean => respond(clean::clean(ctx), json, stdout, stderr),
        MemoryCommand::Query {
            text,
            kind,
            tier,
            tag,
            limit,
            include_expired,
            explain,
        } => respond(
            query::query(
                ctx,
                &query::QueryInput {
                    text,
                    kinds: kind,
                    tier,
                    tag,
                    limit,
                    include_expired,
                    explain,
                },
            ),
            json,
            stdout,
            stderr,
        ),
        MemoryCommand::Lint => {
            let outcome = lint::lint(ctx);
            let failing = outcome
                .as_ref()
                .is_ok_and(|report| report.value.has_errors());
            let code = respond(outcome, json, stdout, stderr);
            if failing { ExitCode::FAILURE } else { code }
        }
        MemoryCommand::Review { feature } => {
            respond(review::review(ctx, feature), json, stdout, stderr)
        }
        MemoryCommand::Reindex => respond(reindex::reindex(ctx), json, stdout, stderr),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/memory_dispatch.rs"]
mod tests;
