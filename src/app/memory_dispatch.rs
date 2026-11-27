use std::io;
use std::process::ExitCode;

use super::respond::respond;
use crate::action::Ctx;
use crate::action::memory::coordination::cli;
use crate::action::memory::{clean, eval, lint, query, reindex, review, status};
use crate::cli::MemoryCommand;
use crate::cli::memory::{EvalCommand, HandoffCommand};

/// The review, read-only or not; clap keeps `--read-only` and a confirmation
/// apart, so a confirmation is never dropped here.
fn review_outcome(
    ctx: &Ctx,
    feature: Option<String>,
    confirm: bool,
    stale: Vec<(String, review::Verdict)>,
    not_required: Option<String>,
    read_only: bool,
) -> crate::error::Outcome<review::ReviewReport> {
    if read_only {
        return review::review_read_only(ctx, feature);
    }
    let confirmation = confirm.then_some(review::Confirmation {
        verdicts: stale,
        not_required,
    });
    review::review_or_confirm(ctx, feature, confirmation)
}

fn status_outcome(
    ctx: &Ctx,
    read_only: bool,
) -> crate::error::Outcome<status::MemoryStatusOutcome> {
    if read_only {
        status::status_read_only(ctx)
    } else {
        status::status(ctx)
    }
}

pub(super) fn dispatch_memory(
    cmd: MemoryCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        MemoryCommand::Capture { .. } | MemoryCommand::Brief { .. } => ExitCode::SUCCESS,
        MemoryCommand::Status { read_only } => {
            respond(status_outcome(ctx, read_only), json, stdout, stderr)
        }
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
        MemoryCommand::Review {
            feature,
            confirm,
            stale,
            not_required,
            read_only,
        } => respond(
            review_outcome(ctx, feature, confirm, stale, not_required, read_only),
            json,
            stdout,
            stderr,
        ),
        MemoryCommand::Eval { command } => match command {
            EvalCommand::Sample { days, limit, out } => {
                respond(eval::sample(ctx, days, limit, out), json, stdout, stderr)
            }
            EvalCommand::Run { file, k } => respond(eval::run(ctx, &file, k), json, stdout, stderr),
        },
        MemoryCommand::Reindex => respond(reindex::reindex(ctx), json, stdout, stderr),
        MemoryCommand::Handoff { command } => match command {
            HandoffCommand::Begin {
                summary,
                pending,
                decision,
                file,
                feature,
                hall,
                to_provider,
                to_session,
                session,
            } => respond(
                cli::begin_from_cli(
                    ctx,
                    &cli::BeginCliInput {
                        summary,
                        pending,
                        decision,
                        file,
                        feature,
                        hall,
                        to_provider,
                        to_session,
                        session,
                    },
                ),
                json,
                stdout,
                stderr,
            ),
            HandoffCommand::List { feature, hall } => respond(
                cli::list_from_cli(ctx, feature.as_deref(), hall),
                json,
                stdout,
                stderr,
            ),
            HandoffCommand::Cancel { id } => {
                respond(cli::cancel_from_cli(ctx, &id), json, stdout, stderr)
            }
        },
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/memory_dispatch.rs"]
mod tests;
