use std::io;
use std::process::ExitCode;

use camino::Utf8PathBuf;

use super::respond::{respond, respond_failure};
use crate::action::Ctx;
use crate::action::discovery::amend as discovery_amend;
use crate::action::discovery::close as discovery_close;
use crate::action::discovery::create as discovery_create;
use crate::action::discovery::list as discovery_list;
use crate::action::discovery::show as discovery_show;
use crate::action::feature::select::resolve_single_feature;
use crate::action::plan::approve::{self as plan_approve};
use crate::action::plan::{
    create as plan_create, lint as plan_lint, list as plan_list, show as plan_show,
    status as plan_status,
};
use crate::cli::root::{DiscoveryCommand, PlanCommand};
use crate::domain::discovery::DiscoveryStatus;

pub(super) fn dispatch_discovery(
    cmd: DiscoveryCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        DiscoveryCommand::Create(args) => respond(
            discovery_create::create(ctx, args.into()),
            json,
            stdout,
            stderr,
        ),
        DiscoveryCommand::List(args) => {
            let status = args.status.as_deref().map(|s| match s {
                "exploring" => DiscoveryStatus::Exploring,
                "converted" => DiscoveryStatus::Converted,
                "abandoned" => DiscoveryStatus::Abandoned,
                _ => DiscoveryStatus::Unknown,
            });
            respond(
                discovery_list::list(ctx, &discovery_list::ListInput { status }),
                json,
                stdout,
                stderr,
            )
        }
        DiscoveryCommand::Show(args) => {
            respond(discovery_show::show(ctx, args.into()), json, stdout, stderr)
        }
        DiscoveryCommand::Amend(args) => {
            let content_res = match args.file.as_deref() {
                Some(path) if path != "-" => {
                    let path = Utf8PathBuf::from(path);
                    match crate::infra::fs::read_text(&path) {
                        Ok(Some(text)) => Ok(text),
                        Ok(None) => Err(crate::error::Failure::blocked(
                            "discovery.file_not_found",
                            format!("file `{path}` not found"),
                        )),
                        Err(err) => Err(err.into()),
                    }
                }
                _ => {
                    use std::io::Read;
                    let mut buf = String::new();
                    match std::io::stdin().read_to_string(&mut buf) {
                        Ok(_) => Ok(buf),
                        Err(e) => Err(crate::error::Failure::failed(
                            "stdin.read_failed",
                            format!("failed to read stdin: {e}"),
                        )),
                    }
                }
            };
            let result = content_res.and_then(|content| {
                let input = discovery_amend::AmendInput {
                    name: args.name,
                    content,
                    merge: args.merge,
                    expected_hash: args.expected_hash,
                    session_id: std::env::var("IVAR_SESSION_ID").ok(),
                };
                discovery_amend::amend(ctx, input)
            });
            respond(result, json, stdout, stderr)
        }
        DiscoveryCommand::Close(args) => {
            let outcome = match args.outcome.as_str() {
                "converted" => DiscoveryStatus::Converted,
                "abandoned" => DiscoveryStatus::Abandoned,
                "exploring" => DiscoveryStatus::Exploring,
                _ => DiscoveryStatus::Unknown,
            };
            let input = discovery_close::CloseInput {
                name: args.name,
                outcome,
            };
            respond(discovery_close::close(ctx, input), json, stdout, stderr)
        }
    }
}

pub(super) fn dispatch_plan(
    cmd: PlanCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        PlanCommand::Create(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to scaffold plans") {
                Ok(feature) => respond(
                    plan_create::create(
                        ctx,
                        plan_create::CreateInput {
                            feature,
                            artifacts: args.artifacts,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        PlanCommand::List => respond(plan_list::list(ctx), json, stdout, stderr),
        PlanCommand::Show(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to show plan") {
                Ok(feature) => respond(
                    plan_show::show(
                        ctx,
                        plan_show::ShowInput {
                            feature,
                            artifact: args.artifact,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        PlanCommand::Approve(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to approve gate") {
                Ok(feature) => respond(
                    plan_approve::approve(
                        ctx,
                        plan_approve::ApproveInput {
                            feature,
                            gate: args.gate,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        PlanCommand::Invalidate(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to invalidate gate") {
                Ok(feature) => respond(
                    plan_approve::invalidate(
                        ctx,
                        plan_approve::InvalidateInput {
                            feature,
                            gate: args.gate,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        PlanCommand::Lint(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to lint") {
                Ok(feature) => respond(
                    plan_lint::lint(ctx, plan_lint::LintInput { feature }),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        PlanCommand::Status(args) => {
            match resolve_single_feature(ctx, args.target, "Select a feature to inspect") {
                Ok(target) => respond(
                    plan_status::status(ctx, &plan_status::StatusInput { plan_path: target }),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
    }
}
