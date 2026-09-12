use serde::Serialize;
use std::io;
use std::process::ExitCode;

use ivar::action::Ctx;
use ivar::action::graph::{
    AffectedInput, CalleesInput, CallersInput, ComplexityInput, DeadCodeInput, ExploreInput,
    FileInput, FindInput, HierarchyInput, ImpactInput, IndexInput, PathInput, ToCompact, VizInput,
    affected_cmd, callees_cmd, callers_cmd, clean_cmd, complexity_cmd, dead_code_cmd,
    execute_view_session, explore_cmd, file_cmd, find_cmd, hierarchy_cmd, impact_cmd, index_cmd,
    mcp_cmd, path_cmd, stats_cmd, view_cmd, viz_cmd,
};
use ivar::cli::graph::GraphCommand;
use ivar::error::{Failure, Outcome, Palette, Report, WriteHuman};
use ivar::infra::term;

fn stderr_palette() -> Palette {
    Palette::from_decision(term::colour_for(term::Stream::Stderr, None))
}

fn write_json(w: &mut impl io::Write, value: &impl Serialize) -> io::Result<()> {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"status":"failed","code":"cli.render_failed","what":"could not render JSON output"}"#
            .to_owned()
    });
    writeln!(w, "{rendered}")
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

fn exit_code_for<T>(report: &Report<T>) -> ExitCode {
    if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

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

fn respond_graph<T>(
    result: Outcome<T>,
    json: bool,
    compact: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode
where
    T: Serialize + WriteHuman + ToCompact,
{
    if json {
        respond(result, true, stdout, stderr)
    } else if compact {
        match result {
            Ok(report) => {
                let _ = writeln!(stdout, "{}", report.value.to_compact());
                ExitCode::SUCCESS
            }
            Err(failure) => respond_failure(failure, false, stdout, stderr),
        }
    } else {
        respond(result, false, stdout, stderr)
    }
}

pub(super) fn dispatch_graph(
    cmd: GraphCommand,
    ctx: &Ctx,
    json: bool,
    compact: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        GraphCommand::Explore(args) => respond_graph(
            explore_cmd(
                ctx,
                ExploreInput {
                    query: args.query,
                    repo: args.repo,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Affected(args) => respond_graph(
            affected_cmd(
                ctx,
                AffectedInput {
                    files: args.files,
                    stdin: args.stdin,
                    repo: args.repo,
                    max_depth: args.max_depth,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Path(args) => respond_graph(
            path_cmd(
                ctx,
                PathInput {
                    from: args.from,
                    to: args.to,
                    max_hops: args.max_hops,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Find(args) => respond_graph(
            find_cmd(
                ctx,
                FindInput {
                    query: args.query,
                    repo: args.repo,
                    limit: args.limit,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Callers(args) => respond_graph(
            callers_cmd(
                ctx,
                CallersInput {
                    symbol: args.symbol,
                    repo: args.repo,
                    cross_repo: args.cross_repo,
                    min_confidence: args.min_confidence,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Callees(args) => respond_graph(
            callees_cmd(
                ctx,
                CalleesInput {
                    symbol_id: args.symbol_id,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::File(args) => respond_graph(
            file_cmd(
                ctx,
                FileInput {
                    repo: args.repo,
                    path: args.path,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Index(args) => respond_graph(
            index_cmd(
                ctx,
                IndexInput {
                    repo: args.repo,
                    full: args.full,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Stats => respond_graph(stats_cmd(ctx), json, compact, stdout, stderr),
        GraphCommand::Impact(args) => respond_graph(
            impact_cmd(
                ctx,
                ImpactInput {
                    symbol_id: args.symbol_id,
                    max_depth: args.max_depth,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::DeadCode(args) => respond_graph(
            dead_code_cmd(
                ctx,
                DeadCodeInput {
                    repo: args.repo,
                    limit: args.limit,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Complexity(args) => respond_graph(
            complexity_cmd(
                ctx,
                ComplexityInput {
                    threshold: args.threshold,
                    repo: args.repo,
                    limit: args.limit,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Hierarchy(args) => respond_graph(
            hierarchy_cmd(
                ctx,
                HierarchyInput {
                    symbol: args.symbol,
                    repo: args.repo,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::Viz(args) => respond_graph(
            viz_cmd(
                ctx,
                VizInput {
                    output: args.output,
                    repo: args.repo,
                },
            ),
            json,
            compact,
            stdout,
            stderr,
        ),
        GraphCommand::View(args) => match view_cmd(ctx, args.into()) {
            Ok(report) => execute_view_session(report.value, json, compact, stdout, stderr),
            Err(failure) => respond_failure(failure, json, stdout, stderr),
        },
        GraphCommand::Clean(args) => {
            respond_graph(clean_cmd(ctx, args.into()), json, compact, stdout, stderr)
        }
        GraphCommand::Mcp(args) => {
            respond_graph(mcp_cmd(ctx, args.tools), json, compact, stdout, stderr)
        }
    }
}
