//! CLI dispatch: turns a parsed [`Cli`] into an exit code by matching
//! `cli.command`, converting each variant's args into its action's `Input`,
//! and rendering the outcome. Dispatch lives here so the binary stays a
//! thin `main` — see ARCHITECTURE.md's module map.

use std::io::{self, Write};
use std::process::ExitCode;

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::batch::run_feature_batch;
use crate::action::confirm;
use crate::action::discovery::amend as discovery_amend;
use crate::action::discovery::close as discovery_close;
use crate::action::discovery::create as discovery_create;
use crate::action::discovery::list as discovery_list;
use crate::action::discovery::show as discovery_show;
use crate::action::execute::{
    accept_revision, checkpoint, finish, interrupt, start, status as execute_status,
};
use crate::action::feature::select::{resolve_multi_features, resolve_single_feature};
use crate::action::feature::{
    cleanup, close, create, delete, deliver, demote, integrate, list as feature_list, promote,
    prune as feature_prune, rebase, rename, reparent, status, view, workspace,
};
use crate::action::hall;
use crate::action::mcp::auth as mcp_auth;
use crate::action::plan::approve::{self as plan_approve};
use crate::action::plan::{
    create as plan_create, list as plan_list, show as plan_show, status as plan_status,
};
use crate::action::provider::{add as provider_add, list as provider_list};
use crate::action::repo::{
    add, create as repo_create, list as repo_list, pull, remove, setup as repo_setup,
    upstream as repo_upstream, view as repo_view,
};
use crate::action::review::comment as review_comment;
use crate::action::session::{
    connect as session_connect, conversion as session_conversion, env_cmd as session_env_cmd,
    guard_cmd as session_guard_cmd, prune as session_prune, relay as session_relay,
    start as session_start, stop as session_stop,
};
use crate::action::skill::{
    add as skill_add, create as skill_create, detach as skill_detach, doctor as skill_doctor,
    list as skill_list, remove as skill_remove, status as skill_status, sync as skill_sync,
    update as skill_update,
};
use crate::action::sync;
use crate::app::respond::{current_dir, respond, respond_batch, respond_failure};
use crate::cli::root::{
    Cli, Command, CommentCommand, DiscoveryCommand, ExecuteCommand, FeatureCommand, McpCommand,
    PlanCommand, ProviderCommand, RepoCommand, ReviewCommand, SessionCommand, SkillCommand,
};
use crate::domain::discovery::DiscoveryStatus;
use crate::error::Report;
use crate::infra::progress;
use crate::infra::term;

#[expect(
    clippy::too_many_lines,
    reason = "flat match over Command subcommands — each arm parses CLI args \
              into one *Input and delegates to one action fn's public entry \
              point; splitting the match into ad hoc helpers would only \
              relocate, not reduce, the branching"
)]
pub fn run(cli: Cli) -> ExitCode {
    let json = cli.json;
    let compact = cli.compact;
    // Prime both per-stream colour decisions with the flag, before any output
    // exists to render. `term`'s caches take their value from the first call
    // and ignore the argument afterwards, which is what lets `respond` and
    // `respond_failure` ask for the answer without every one of the ~60
    // dispatch arms below having to carry a palette down to them.
    //
    // Both streams are primed because they are redirected independently: the
    // value goes to stdout, failures and warnings to stderr.
    let _ = term::colour_for(term::Stream::Stdout, cli.color.as_override());
    let _ = term::colour_for(term::Stream::Stderr, cli.color.as_override());

    // The progress sink, decided once for the same reason the colour caches are
    // primed above: `--json` is a machine-shaped run and wants no redraw line
    // even on stderr. `progress::reporter` asks the is-it-a-tty half.
    //
    // The confirmation seam is decided by the same rule: a `--json` run, a
    // `$CI` run, or a run with nobody on stderr may not prompt — a pipe is not
    // consent. Everything that must ask before it acts (`cleanup`, `migrate`,
    // and later integration's parent-promotion prompt) reads this one decision.
    let ctx = Ctx::new(current_dir())
        .with_progress(progress::reporter(!json))
        .with_confirm(confirm::reporter(
            !json
                && std::env::var_os("CI").is_none()
                && term::is_tty(term::Stream::Stderr)
                && term::is_tty(term::Stream::Stdin),
        ));

    let session_id = std::env::var("IVAR_SESSION_ID").ok();

    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();

    match cli.command {
        Command::Init(args) => respond(
            hall::init(&ctx, &args.into()),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Sync(args) => respond(
            sync::sync(&ctx, &args.into()),
            json,
            &mut stdout,
            &mut stderr,
        ),
        Command::Status => respond(hall::status(&ctx), json, &mut stdout, &mut stderr),
        Command::Doctor => respond(hall::doctor(&ctx), json, &mut stdout, &mut stderr),
        Command::Cleanup => respond(hall::cleanup(&ctx), json, &mut stdout, &mut stderr),
        Command::Migrate => respond(hall::migrate(&ctx), json, &mut stdout, &mut stderr),
        Command::Repo(cmd) => match cmd {
            RepoCommand::List => respond(repo_list::list(&ctx), json, &mut stdout, &mut stderr),
            RepoCommand::Add(args) => {
                respond(add::add(&ctx, args.into()), json, &mut stdout, &mut stderr)
            }
            RepoCommand::Create(args) => respond(
                repo_create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Remove(args) => respond(
                remove::remove(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Pull(args) => respond(
                pull::pull(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Setup(args) => respond(
                repo_setup::setup(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::View(args) => respond(
                repo_view::view(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            RepoCommand::Upstream(args) => respond(
                repo_upstream::upstream(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Feature(cmd) => match cmd {
            FeatureCommand::Create(args) => respond(
                create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            FeatureCommand::List => {
                respond(feature_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
            FeatureCommand::Promote(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to promote into")
                {
                    Ok(feature) => respond(
                        promote::promote(
                            &ctx,
                            promote::PromoteInput {
                                feature,
                                repo: args.repo,
                                base: args.base,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Demote(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to demote from")
                {
                    Ok(feature) => respond(
                        demote::demote(
                            &ctx,
                            demote::DemoteInput {
                                feature,
                                repo: args.repo,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Status(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to inspect") {
                    Ok(feature) => respond(
                        status::status(
                            &ctx,
                            status::StatusInput {
                                feature,
                                recursive: args.recursive,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Reparent(args) => {
                match resolve_single_feature(&ctx, args.child, "Select a child feature to reparent")
                {
                    Ok(child) => respond(
                        reparent::reparent(
                            &ctx,
                            reparent::ReparentInput {
                                child,
                                parent: args.parent,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Rename(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to rename") {
                    Ok(feature) => respond(
                        rename::rename(
                            &ctx,
                            rename::RenameInput {
                                feature,
                                name: args.name,
                                branch: args.branch,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Integrate(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to integrate") {
                    Ok(feature) => respond(
                        integrate::integrate(
                            &ctx,
                            integrate::IntegrateInput {
                                feature,
                                via: args.via,
                                strategy: args.strategy,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Execute(cmd) => match cmd {
                ExecuteCommand::Start(args) => {
                    match resolve_single_feature(
                        &ctx,
                        args.feature,
                        "Select a feature to start execution",
                    ) {
                        Ok(feature) => respond(
                            start::start(
                                &ctx,
                                start::StartInput {
                                    feature,
                                    plan: args.plan,
                                    resume: args.resume,
                                    restart: args.restart,
                                },
                            ),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                    }
                }
                ExecuteCommand::Finish(args) if args.print_schema => respond(
                    Ok(Report::new(finish::ReportSchema::current())),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                ExecuteCommand::Finish(args) => {
                    let input = resolve_single_feature(
                        &ctx,
                        args.feature.clone(),
                        "Select a feature to finish execution",
                    )
                    .and_then(|feature| {
                        finish::FinishInput::try_from(args)
                            .map(|input| finish::FinishInput { feature, ..input })
                    });
                    match input {
                        Ok(input) => {
                            respond(finish::finish(&ctx, input), json, &mut stdout, &mut stderr)
                        }
                        Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                    }
                }
                ExecuteCommand::Status(args) => {
                    match resolve_single_feature(
                        &ctx,
                        args.feature,
                        "Select a feature to check execution status",
                    ) {
                        Ok(feature) => respond(
                            execute_status::status(
                                &ctx,
                                execute_status::StatusInput {
                                    feature,
                                    plan: args.plan,
                                    history: args.history,
                                    run: args.run,
                                },
                            ),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                    }
                }
                ExecuteCommand::AcceptRevision(args) => {
                    match resolve_single_feature(
                        &ctx,
                        args.feature,
                        "Select a feature to accept revision",
                    ) {
                        Ok(feature) => respond(
                            accept_revision::accept_revision(
                                &ctx,
                                accept_revision::AcceptRevisionInput {
                                    feature,
                                    plan: args.plan,
                                },
                            ),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                    }
                }
                ExecuteCommand::Checkpoint(args) => {
                    match resolve_single_feature(
                        &ctx,
                        args.feature,
                        "Select a feature to record a wave checkpoint",
                    ) {
                        Ok(feature) => respond(
                            checkpoint::checkpoint(
                                &ctx,
                                checkpoint::CheckpointInput {
                                    feature,
                                    wave: args.wave,
                                    summary: args.summary,
                                },
                            ),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                    }
                }
                ExecuteCommand::Interrupt(args) => {
                    match resolve_single_feature(
                        &ctx,
                        args.feature,
                        "Select a feature to interrupt execution",
                    ) {
                        Ok(feature) => respond(
                            interrupt::interrupt(&ctx, interrupt::InterruptInput { feature }),
                            json,
                            &mut stdout,
                            &mut stderr,
                        ),
                        Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                    }
                }
            },
            FeatureCommand::Deliver(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to deliver") {
                    Ok(feature) => respond(
                        deliver::deliver(
                            &ctx,
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
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Close(args) => match args.name {
                Some(name) => respond(
                    close::close(
                        &ctx,
                        close::CloseInput {
                            name,
                            outcome: args.outcome,
                        },
                    ),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                None => match resolve_multi_features(&ctx, None, "Select features to close") {
                    Ok(targets) => {
                        let items = run_feature_batch(&targets, 4, |f| {
                            close::close(
                                &ctx,
                                close::CloseInput {
                                    name: f.to_owned(),
                                    outcome: args.outcome.clone(),
                                },
                            )
                        });
                        respond_batch(items, json, &mut stdout, &mut stderr)
                    }
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                },
            },
            FeatureCommand::Delete(args) => match args.name {
                Some(name) => respond(
                    delete::delete(&ctx, delete::DeleteInput { name }),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                None => match resolve_multi_features(&ctx, None, "Select features to delete") {
                    Ok(targets) => {
                        let items = run_feature_batch(&targets, 4, |f| {
                            delete::delete(&ctx, delete::DeleteInput { name: f.to_owned() })
                        });
                        respond_batch(items, json, &mut stdout, &mut stderr)
                    }
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                },
            },
            FeatureCommand::Cleanup(args) => {
                let input = |feature: String| cleanup::CleanupInput {
                    feature,
                    preview: args.preview,
                    record: args.record.clone(),
                    session_id: session_id.clone(),
                };
                match args.name.clone() {
                    Some(feature) => respond(
                        cleanup::cleanup(&ctx, input(feature)),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    None => {
                        match resolve_multi_features(&ctx, None, "Select features to clean up") {
                            Ok(targets) => {
                                let items = run_feature_batch(&targets, 4, |f| {
                                    cleanup::cleanup(&ctx, input(f.to_owned()))
                                });
                                respond_batch(items, json, &mut stdout, &mut stderr)
                            }
                            Err(failure) => {
                                respond_failure(&failure, json, &mut stdout, &mut stderr)
                            }
                        }
                    }
                }
            }
            FeatureCommand::Workspace(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature for workspace") {
                    Ok(feature) => respond(
                        workspace::workspace(
                            &ctx,
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
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Rebase(args) => match args.name {
                Some(name) => respond(
                    rebase::rebase(
                        &ctx,
                        rebase::RebaseInput {
                            name,
                            onto: args.onto,
                        },
                    ),
                    json,
                    &mut stdout,
                    &mut stderr,
                ),
                None => match resolve_multi_features(&ctx, None, "Select features to rebase") {
                    Ok(targets) => {
                        let items = run_feature_batch(&targets, 4, |f| {
                            rebase::rebase(
                                &ctx,
                                rebase::RebaseInput {
                                    name: f.to_owned(),
                                    onto: args.onto.clone(),
                                },
                            )
                        });
                        respond_batch(items, json, &mut stdout, &mut stderr)
                    }
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                },
            },
            FeatureCommand::View(args) => {
                match resolve_single_feature(&ctx, args.name, "Select a feature to view") {
                    Ok(feature) => respond(
                        view::view(&ctx, view::ViewInput { feature }),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            FeatureCommand::Prune => {
                respond(feature_prune::prune(&ctx), json, &mut stdout, &mut stderr)
            }
        },
        Command::Session(cmd) => match cmd {
            SessionCommand::Start(args) => respond(
                session_start::start(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Connect(args) => respond(
                session_connect::connect(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Convert(args) => respond(
                session_conversion::convert(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Stop(args) => respond(
                session_stop::stop(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Prune => {
                respond(session_prune::prune(&ctx), json, &mut stdout, &mut stderr)
            }
            SessionCommand::Relay(args) => respond(
                session_relay::relay(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Env(args) => respond(
                session_env_cmd::run(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SessionCommand::Sandbox(args) => {
                match crate::action::session::sandbox::run_launcher(
                    &ctx,
                    &args.session,
                    args.resume,
                    &args.command,
                ) {
                    Ok(()) => std::process::ExitCode::SUCCESS,
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
        },
        Command::Provider(cmd) => match cmd {
            ProviderCommand::List => {
                respond(provider_list::list(&ctx), json, &mut stdout, &mut stderr)
            }
            ProviderCommand::Add(args) => respond(
                provider_add::add(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Discovery(cmd) => match cmd {
            DiscoveryCommand::Create(args) => respond(
                discovery_create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            DiscoveryCommand::List(args) => {
                let status = args.status.as_deref().map(|s| match s {
                    "exploring" => DiscoveryStatus::Exploring,
                    "converted" => DiscoveryStatus::Converted,
                    "abandoned" => DiscoveryStatus::Abandoned,
                    _ => DiscoveryStatus::Unknown,
                });
                respond(
                    discovery_list::list(&ctx, &discovery_list::ListInput { status }),
                    json,
                    &mut stdout,
                    &mut stderr,
                )
            }
            DiscoveryCommand::Show(args) => respond(
                discovery_show::show(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
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
                    discovery_amend::amend(&ctx, input)
                });
                respond(result, json, &mut stdout, &mut stderr)
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
                respond(
                    discovery_close::close(&ctx, input),
                    json,
                    &mut stdout,
                    &mut stderr,
                )
            }
        },
        Command::Review(ReviewCommand::Comment(cmd)) => match cmd {
            CommentCommand::Add(args) => respond(
                review_comment::add(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            CommentCommand::List(args) => respond(
                review_comment::list(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            CommentCommand::Resolve(args) => respond(
                review_comment::resolve(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Plan(cmd) => match cmd {
            PlanCommand::Create(args) => {
                match resolve_single_feature(
                    &ctx,
                    args.feature,
                    "Select a feature to scaffold plans",
                ) {
                    Ok(feature) => respond(
                        plan_create::create(
                            &ctx,
                            plan_create::CreateInput {
                                feature,
                                artifacts: args.artifacts,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            PlanCommand::List => respond(plan_list::list(&ctx), json, &mut stdout, &mut stderr),
            PlanCommand::Show(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to show plan") {
                    Ok(feature) => respond(
                        plan_show::show(
                            &ctx,
                            plan_show::ShowInput {
                                feature,
                                artifact: args.artifact,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            PlanCommand::Approve(args) => {
                match resolve_single_feature(&ctx, args.feature, "Select a feature to approve gate")
                {
                    Ok(feature) => respond(
                        plan_approve::approve(
                            &ctx,
                            plan_approve::ApproveInput {
                                feature,
                                gate: args.gate,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            PlanCommand::Invalidate(args) => {
                match resolve_single_feature(
                    &ctx,
                    args.feature,
                    "Select a feature to invalidate gate",
                ) {
                    Ok(feature) => respond(
                        plan_approve::invalidate(
                            &ctx,
                            plan_approve::InvalidateInput {
                                feature,
                                gate: args.gate,
                            },
                        ),
                        json,
                        &mut stdout,
                        &mut stderr,
                    ),
                    Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
                }
            }
            PlanCommand::Status(args) => respond(
                plan_status::status(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Skill(cmd) => match cmd {
            SkillCommand::List => respond(skill_list::list(&ctx), json, &mut stdout, &mut stderr),
            SkillCommand::Create(args) => respond(
                skill_create::create(&ctx, args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Add(args) => respond(
                skill_add::add(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Update(args) => respond(
                skill_update::update(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Remove(args) => respond(
                skill_remove::remove(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Detach(args) => respond(
                skill_detach::detach(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
            SkillCommand::Sync => respond(skill_sync::sync(&ctx), json, &mut stdout, &mut stderr),
            SkillCommand::Status => {
                respond(skill_status::status(&ctx), json, &mut stdout, &mut stderr)
            }
            SkillCommand::Doctor => {
                respond(skill_doctor::doctor(&ctx), json, &mut stdout, &mut stderr)
            }
        },
        Command::Guard(args) => {
            let input = match session_guard_cmd::GuardInput::try_from(args) {
                Ok(input) => input,
                Err(failure) => {
                    return respond_failure(&failure, json, &mut stdout, &mut stderr);
                }
            };
            match session_guard_cmd::run(&input) {
                Ok(outcome) => {
                    if !outcome.body.is_empty() {
                        let _ = write!(stdout, "{}", outcome.body);
                    }
                    if outcome.exit_zero {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(failure) => respond_failure(&failure, json, &mut stdout, &mut stderr),
            }
        }
        Command::Mcp(cmd) => match cmd {
            McpCommand::Auth(args) => respond(
                mcp_auth::auth(&ctx, &args.into()),
                json,
                &mut stdout,
                &mut stderr,
            ),
        },
        Command::Graph(cmd) => super::graph_dispatch::dispatch_graph(
            cmd,
            &ctx,
            json,
            compact,
            &mut stdout,
            &mut stderr,
        ),
        // Git's credential protocol is raw on stdin/stdout — it must not pass
        // through `respond`, which would render a `Report` on top of it.
        Command::GitCredential(args) => {
            match crate::git::credential::run(args.operation.as_deref()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    let _ = writeln!(io::stderr().lock(), "ivar: git-credential: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
