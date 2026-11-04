use std::io;
use std::process::ExitCode;

use super::respond::{respond, respond_batch, respond_failure};
use crate::action::Ctx;
use crate::action::batch::run_feature_batch;
use crate::action::execute::{
    accept_revision, checkpoint, finish, interrupt, start, status as execute_status,
};
use crate::action::feature::select::{resolve_multi_features, resolve_single_feature};
use crate::action::feature::{
    cleanup, close, create, delete, deliver, demote, integrate, list as feature_list, promote,
    prune as feature_prune, rebase, rename, reparent, status, view, workspace,
};
use crate::cli::root::{ExecuteCommand, FeatureCommand};
use crate::error::Report;

#[expect(
    clippy::too_many_lines,
    reason = "flat match over FeatureCommand subcommands — each arm resolves the feature \
              and delegates to one action fn; splitting it would only relocate the branching"
)]
pub(super) fn dispatch_feature(
    cmd: FeatureCommand,
    ctx: &Ctx,
    json: bool,
    session_id: Option<&str>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        FeatureCommand::Create(args) => {
            respond(create::create(ctx, args.into()), json, stdout, stderr)
        }
        FeatureCommand::List => respond(feature_list::list(ctx), json, stdout, stderr),
        FeatureCommand::Promote(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to promote into") {
                Ok(feature) => respond(
                    promote::promote(
                        ctx,
                        promote::PromoteInput {
                            feature,
                            repo: args.repo,
                            base: args.base,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Demote(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to demote from") {
                Ok(feature) => respond(
                    demote::demote(
                        ctx,
                        demote::DemoteInput {
                            feature,
                            repo: args.repo,
                            remove_worktree: args.remove_worktree,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Status(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to inspect") {
                Ok(feature) => respond(
                    status::status(
                        ctx,
                        status::StatusInput {
                            feature,
                            recursive: args.recursive,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Reparent(args) => {
            match resolve_single_feature(ctx, args.child, "Select a child feature to reparent") {
                Ok(child) => respond(
                    reparent::reparent(
                        ctx,
                        reparent::ReparentInput {
                            child,
                            parent: args.parent,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Rename(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to rename") {
                Ok(feature) => respond(
                    rename::rename(
                        ctx,
                        rename::RenameInput {
                            feature,
                            name: args.name,
                            branch: args.branch,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Integrate(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to integrate") {
                Ok(feature) => respond(
                    integrate::integrate(
                        ctx,
                        integrate::IntegrateInput {
                            feature,
                            via: args.via,
                            strategy: args.strategy,
                            name: args.name,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Execute(cmd) => match cmd {
            ExecuteCommand::Start(args) => {
                match resolve_single_feature(
                    ctx,
                    args.feature,
                    "Select a feature to start execution",
                ) {
                    Ok(feature) => respond(
                        start::start(
                            ctx,
                            start::StartInput {
                                feature,
                                plan: args.plan,
                                resume: args.resume,
                                restart: args.restart,
                                mode: args.mode,
                                session: session_id.map(str::to_owned),
                            },
                        ),
                        json,
                        stdout,
                        stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                }
            }
            ExecuteCommand::Finish(args) if args.print_schema => respond(
                Ok(Report::new(finish::ReportSchema::current())),
                json,
                stdout,
                stderr,
            ),
            ExecuteCommand::Finish(args) => {
                let input = resolve_single_feature(
                    ctx,
                    args.feature.clone(),
                    "Select a feature to finish execution",
                )
                .and_then(|feature| {
                    finish::FinishInput::try_from(args)
                        .map(|input| finish::FinishInput { feature, ..input })
                });
                match input {
                    Ok(input) => respond(finish::finish(ctx, input), json, stdout, stderr),
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                }
            }
            ExecuteCommand::Status(args) => {
                match resolve_single_feature(
                    ctx,
                    args.feature,
                    "Select a feature to check execution status",
                ) {
                    Ok(feature) => respond(
                        execute_status::status(
                            ctx,
                            execute_status::StatusInput {
                                feature,
                                plan: args.plan,
                                history: args.history,
                                run: args.run,
                            },
                        ),
                        json,
                        stdout,
                        stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                }
            }
            ExecuteCommand::AcceptRevision(args) => {
                match resolve_single_feature(
                    ctx,
                    args.feature,
                    "Select a feature to accept revision",
                ) {
                    Ok(feature) => respond(
                        accept_revision::accept_revision(
                            ctx,
                            accept_revision::AcceptRevisionInput {
                                feature,
                                plan: args.plan,
                            },
                        ),
                        json,
                        stdout,
                        stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                }
            }
            ExecuteCommand::Checkpoint(args) => {
                match resolve_single_feature(
                    ctx,
                    args.feature,
                    "Select a feature to record a wave checkpoint",
                ) {
                    Ok(feature) => respond(
                        checkpoint::checkpoint(
                            ctx,
                            checkpoint::CheckpointInput {
                                feature,
                                wave: args.wave,
                                summary: args.summary,
                            },
                        ),
                        json,
                        stdout,
                        stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                }
            }
            ExecuteCommand::Interrupt(args) => {
                match resolve_single_feature(
                    ctx,
                    args.feature,
                    "Select a feature to interrupt execution",
                ) {
                    Ok(feature) => respond(
                        interrupt::interrupt(ctx, interrupt::InterruptInput { feature }),
                        json,
                        stdout,
                        stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                }
            }
        },
        FeatureCommand::Deliver(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature to deliver") {
                Ok(feature) => respond(
                    deliver::deliver(
                        ctx,
                        deliver::DeliverInput {
                            feature,
                            preview: args.preview,
                            land: args.land,
                            fingerprint: args.fingerprint,
                            global_metadata: args.global_metadata,
                            repo_overrides: args.repo_overrides,
                            only: args.only,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Close(args) => match args.name {
            Some(name) => respond(
                close::close(
                    ctx,
                    close::CloseInput {
                        name,
                        outcome: args.outcome,
                    },
                ),
                json,
                stdout,
                stderr,
            ),
            None => match resolve_multi_features(ctx, None, "Select features to close") {
                Ok(targets) => {
                    let items = run_feature_batch(&targets, 4, |f| {
                        close::close(
                            ctx,
                            close::CloseInput {
                                name: f.to_owned(),
                                outcome: args.outcome.clone(),
                            },
                        )
                    });
                    respond_batch(items, json, stdout, stderr)
                }
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            },
        },
        FeatureCommand::Delete(args) => {
            let descendants = if args.yes {
                delete::Descendants::Consented
            } else {
                delete::Descendants::Ask
            };
            match args.name {
                Some(name) => respond(
                    delete::delete(
                        ctx,
                        delete::DeleteInput {
                            name,
                            force: args.force,
                            descendants,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                None => match resolve_multi_features(ctx, None, "Select features to delete") {
                    Ok(targets) => match delete::fold_into_ancestors(ctx, &targets) {
                        Ok(roots) => {
                            // Sequential batch execution (batch size 1) to avoid concurrent prompts or races
                            let items = run_feature_batch(&roots, 1, |f| {
                                delete::delete(
                                    ctx,
                                    delete::DeleteInput {
                                        name: f.to_owned(),
                                        force: args.force,
                                        descendants,
                                    },
                                )
                            });
                            respond_batch(items, json, stdout, stderr)
                        }
                        Err(failure) => respond_failure(&failure, json, stdout, stderr),
                    },
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                },
            }
        }
        FeatureCommand::Cleanup(args) => {
            let input = |feature: String| cleanup::CleanupInput {
                feature,
                preview: args.preview,
                record: args.record.clone(),
                session_id: session_id.map(str::to_owned),
            };
            match args.name.clone() {
                Some(feature) => {
                    respond(cleanup::cleanup(ctx, input(feature)), json, stdout, stderr)
                }
                None => match resolve_multi_features(ctx, None, "Select features to clean up") {
                    Ok(targets) => {
                        let items = run_feature_batch(&targets, 4, |f| {
                            cleanup::cleanup(ctx, input(f.to_owned()))
                        });
                        respond_batch(items, json, stdout, stderr)
                    }
                    Err(failure) => respond_failure(&failure, json, stdout, stderr),
                },
            }
        }
        FeatureCommand::Workspace(args) => {
            match resolve_single_feature(ctx, args.feature, "Select a feature for workspace") {
                Ok(feature) => respond(
                    workspace::workspace(
                        ctx,
                        // The only arm that consults `json` for anything but
                        // rendering: opening an editor is a human convenience, and
                        // a machine-shaped run has no window to open into. `From`
                        // cannot see the flag, so it is applied here.
                        workspace::WorkspaceInput {
                            feature,
                            repos: args.repos,
                            open: !json,
                        },
                    ),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Rebase(args) => match args.name {
            Some(name) => respond(
                rebase::rebase(
                    ctx,
                    rebase::RebaseInput {
                        name,
                        onto: args.onto,
                        repos: args.repos,
                        offline: args.offline,
                    },
                ),
                json,
                stdout,
                stderr,
            ),
            None => match resolve_multi_features(ctx, None, "Select features to rebase") {
                Ok(targets) => {
                    let items = run_feature_batch(&targets, 4, |f| {
                        rebase::rebase(
                            ctx,
                            rebase::RebaseInput {
                                name: f.to_owned(),
                                onto: args.onto.clone(),
                                repos: args.repos.clone(),
                                offline: args.offline,
                            },
                        )
                    });
                    respond_batch(items, json, stdout, stderr)
                }
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            },
        },
        FeatureCommand::View(args) => {
            match resolve_single_feature(ctx, args.name, "Select a feature to view") {
                Ok(feature) => respond(
                    view::view(ctx, view::ViewInput { feature }),
                    json,
                    stdout,
                    stderr,
                ),
                Err(failure) => respond_failure(&failure, json, stdout, stderr),
            }
        }
        FeatureCommand::Prune => respond(feature_prune::prune(ctx), json, stdout, stderr),
    }
}
