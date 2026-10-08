#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::CommandFactory as _;
use rstest::rstest;

use super::*;
use crate::action::upgrade::command::UpgradeInput;

#[test]
fn feature_deliver_parses_grouped_metadata_and_preserves_order() {
    let cli = Cli::try_parse_from([
        "ivar",
        "feature",
        "deliver",
        "checkout",
        "--name",
        "feat: global title",
        "--body",
        "global body",
        "--repo",
        "api",
        "--name",
        "feat(api): title",
        "--body",
        "./docs/api.md",
        "--repo",
        "web",
        "--name",
        "feat(web): title",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Deliver(args)) => {
            let input: deliver::DeliverInput = args.into();
            assert_eq!(input.feature, "checkout");
            assert_eq!(
                input.global_metadata,
                deliver::PullRequestMetadata {
                    title: Some("feat: global title".to_owned()),
                    body: Some("global body".to_owned()),
                    draft: None,
                }
            );
            assert_eq!(
                input.repo_overrides,
                vec![
                    deliver::RepoMetadataOverride {
                        repo: "api".to_owned(),
                        metadata: deliver::PullRequestMetadata {
                            title: Some("feat(api): title".to_owned()),
                            body: Some("./docs/api.md".to_owned()),
                            draft: None,
                        },
                    },
                    deliver::RepoMetadataOverride {
                        repo: "web".to_owned(),
                        metadata: deliver::PullRequestMetadata {
                            title: Some("feat(web): title".to_owned()),
                            body: None,
                            draft: None,
                        },
                    },
                ]
            );
        }
        other => panic!("expected feature deliver, got {other:?}"),
    }
}

#[test]
fn feature_deliver_rejects_duplicate_field_in_same_scope() {
    let global_dup = Cli::try_parse_from([
        "ivar", "feature", "deliver", "checkout", "--name", "title 1", "--name", "title 2",
    ]);
    assert!(global_dup.is_err());

    let repo_dup = Cli::try_parse_from([
        "ivar", "feature", "deliver", "checkout", "--repo", "api", "--body", "body 1", "--body",
        "body 2",
    ]);
    assert!(repo_dup.is_err());
}

#[test]
fn feature_deliver_parses_legacy_arguments_without_metadata() {
    let cli = Cli::try_parse_from(["ivar", "feature", "deliver", "checkout", "--preview"]).unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Deliver(args)) => {
            let input: deliver::DeliverInput = args.into();
            assert_eq!(input.feature, "checkout");
            assert!(input.preview);
            assert_eq!(
                input.global_metadata,
                deliver::PullRequestMetadata::default()
            );
            assert!(input.repo_overrides.is_empty());
        }
        other => panic!("expected feature deliver, got {other:?}"),
    }
}

#[test]
fn cli_definition_is_valid() {
    // `debug_assert` panics on a malformed clap definition (duplicate
    // ids, conflicting args, ...) — the cheapest test that the derive
    // actually produced a usable `Command`.
    Cli::command().debug_assert();
}

#[rstest]
#[case::git_credential_get(&["ivar", "git-credential", "get"], |command: &Command| matches!(command, Command::GitCredential(args) if args.operation.as_deref() == Some("get")))]
#[case::git_credential_store(&["ivar", "git-credential", "store"], |command: &Command| matches!(command, Command::GitCredential(args) if args.operation.as_deref() == Some("store")))]
#[case::git_credential_erase(&["ivar", "git-credential", "erase"], |command: &Command| matches!(command, Command::GitCredential(args) if args.operation.as_deref() == Some("erase")))]
// gitcredentials(7): the helper must ignore an operation it does not implement, so clap must not reject it first.
#[case::git_credential_unknown_operation(&["ivar", "git-credential", "capability"], |command: &Command| matches!(command, Command::GitCredential(args) if args.operation.as_deref() == Some("capability")))]
#[case::feature_create_base(&["ivar", "feature", "create", "checkout", "--base", "develop"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Create(args)) if args.base.as_deref() == Some("develop")))]
#[case::feature_create_parent_via_strategy(&["ivar", "feature", "create", "child", "--parent", "parent", "--via", "pr", "--strategy", "rebase"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Create(args)) if args.parent.as_deref() == Some("parent") && args.via.as_deref() == Some("pr") && args.strategy.as_deref() == Some("rebase")))]
#[case::feature_reparent_child_and_parent(&["ivar", "feature", "reparent", "child", "--parent", "new-parent"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Reparent(args)) if args.child.as_deref() == Some("child") && args.parent == "new-parent"))]
#[case::feature_status_recursive(&["ivar", "feature", "status", "parent", "--recursive"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Status(args)) if args.recursive))]
#[case::feature_integrate_via_strategy(&["ivar", "feature", "integrate", "child", "--via", "pr", "--strategy", "rebase"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Integrate(args)) if args.feature.as_deref() == Some("child") && args.via.as_deref() == Some("pr") && args.strategy.as_deref() == Some("rebase")))]
#[case::feature_rebase_onto(&["ivar", "feature", "rebase", "checkout", "--onto", "main"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Rebase(args)) if args.onto.as_deref() == Some("main")))]
#[case::feature_deliver_global_draft(&["ivar", "feature", "deliver", "checkout", "--draft"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Deliver(args)) if args.global_metadata.draft == Some(true)))]
#[case::session_sandbox_trailing_argv(&["ivar", "session", "sandbox", "--session", "6f0c9d5f-0000-4000-8000-000000000000", "--", "claude", "--resume"], |command: &Command| matches!(command, Command::Session(SessionCommand::Sandbox(args)) if args.session == "6f0c9d5f-0000-4000-8000-000000000000" && args.command == ["claude", "--resume"]))]
#[case::session_relay_without_feature(&["ivar", "session", "relay", "--provider", "claude-code"], |command: &Command| matches!(command, Command::Session(SessionCommand::Relay(args)) if args.feature.is_none() && args.provider == "claude-code"))]
#[case::session_relay_with_feature(&["ivar", "session", "relay", "feat-a", "--provider", "claude-code"], |command: &Command| matches!(command, Command::Session(SessionCommand::Relay(args)) if args.feature.as_deref() == Some("feat-a") && args.provider == "claude-code"))]
#[case::review_comment_add(&["ivar", "review", "comment", "add", "checkout", "--repo", "api", "--file", "src/lib.rs", "--lines", "3-5", "--body", "- rename this"], |command: &Command| matches!(command, Command::Review(ReviewCommand::Comment(CommentCommand::Add(args))) if args.feature.as_deref() == Some("checkout") && args.lines == "3-5" && args.body == "- rename this"))]
#[case::review_comment_resolve_without_feature(&["ivar", "review", "comment", "resolve", "c1"], |command: &Command| matches!(command, Command::Review(ReviewCommand::Comment(CommentCommand::Resolve(args))) if args.feature.is_none() && args.id == "c1"))]
#[case::execute_finish_without_plan(&["ivar", "feature", "execute", "finish", "checkout", "--report-json", "r.json", "--outcome", "succeeded"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Execute(ExecuteCommand::Finish(args))) if args.plan.is_none()))]
#[case::execute_accept_revision_without_plan(&["ivar", "feature", "execute", "accept-revision", "checkout"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Execute(ExecuteCommand::AcceptRevision(args))) if args.plan.is_none()))]
#[case::execute_status_with_plan(&["ivar", "feature", "execute", "status", "checkout", "--plan", "custom/plan.md"], |command: &Command| matches!(command, Command::Feature(FeatureCommand::Execute(ExecuteCommand::Status(args))) if args.plan.as_deref() == Some("custom/plan.md")))]
#[case::mcp_status_flags(&["ivar", "mcp", "status", "figma", "--provider", "claude-code", "--live"], |command: &Command| matches!(command, Command::Mcp(McpCommand::Status(args)) if args.server.as_deref() == Some("figma") && args.provider.as_deref() == Some("claude-code") && args.live))]
#[case::mcp_status_defaults(&["ivar", "mcp", "status"], |command: &Command| matches!(command, Command::Mcp(McpCommand::Status(args)) if args.server.is_none() && args.provider.is_none() && !args.live))]
fn cli_parses_argv_into_command(#[case] argv: &[&str], #[case] expected: fn(&Command) -> bool) {
    let cli = Cli::try_parse_from(argv).unwrap_or_else(|error| panic!("{argv:?} refused: {error}"));

    assert!(
        expected(&cli.command),
        "{argv:?} parsed as {:?}",
        cli.command
    );
}

#[test]
fn execute_status_rejects_history_with_a_specific_run() {
    let error = Cli::try_parse_from([
        "ivar",
        "feature",
        "execute",
        "status",
        "checkout",
        "--history",
        "--run",
        "00000000-0000-0000-0000-000000000001",
    ])
    .unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

/// `--provider` picks one harness; `--all-providers` runs every harness the
/// hall lists. Naming both at once is not "pick one for me" — it is a
/// contradiction, so clap's `conflicts_with` refuses it outright rather than
/// letting `action::mcp::auth` guess which one wins.
#[test]
fn mcp_auth_rejects_provider_with_all_providers() {
    let error = Cli::try_parse_from([
        "ivar",
        "mcp",
        "auth",
        "figma",
        "--provider",
        "opencode",
        "--all-providers",
    ])
    .unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn init_args_convert_into_init_input_without_change() {
    let args = InitArgs {
        path: Utf8PathBuf::from("some/dir"),
        name: Some("acme".to_owned()),
        provider: Some("opencode".to_owned()),
    };

    let input: InitInput = args.into();

    assert_eq!(input.path, Utf8PathBuf::from("some/dir"));
    assert_eq!(input.name, Some("acme".to_owned()));
    assert_eq!(input.provider, Some("opencode".to_owned()));
}

#[test]
fn feature_cleanup_rejects_both_modes() {
    let error = Cli::try_parse_from([
        "ivar",
        "feature",
        "cleanup",
        "checkout",
        "--preview",
        "--record",
        "docs/updates/001-checkout.cleanup.json",
    ])
    .unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn color_mode_maps_to_the_override_colour_expects() {
    assert_eq!(ColorMode::Auto.as_override(), None);
    assert_eq!(ColorMode::Always.as_override(), Some(true));
    assert_eq!(ColorMode::Never.as_override(), Some(false));
}

/// `--base` and `--parent` are two answers to the same question — where the
/// child's work starts from — and clap refuses both together.
#[test]
fn feature_create_refuses_base_alongside_parent() {
    let error = Cli::try_parse_from([
        "ivar", "feature", "create", "child", "--parent", "parent", "--base", "main",
    ])
    .unwrap_err();

    assert!(
        error.to_string().contains("cannot be used with"),
        "was: {error}"
    );
}

#[test]
fn feature_create_args_convert_into_create_input_without_change() {
    let args = FeatureCreateArgs {
        name: "checkout".to_owned(),
        branch: Some("feat/checkout".to_owned()),
        base: Some("develop".to_owned()),
        parent: None,
        via: None,
        strategy: None,
    };

    let input: crate::action::feature::create::CreateInput = args.into();

    assert_eq!(input.name, "checkout");
    assert_eq!(input.branch, Some("feat/checkout".to_owned()));
    assert_eq!(input.base, Some("develop".to_owned()));
    assert_eq!(input.parent, None);
    assert_eq!(input.via, None);
    assert_eq!(input.strategy, None);
}

/// Reparenting is meaningless without a target: `--parent` is required.
#[test]
fn feature_reparent_requires_a_parent() {
    let error = Cli::try_parse_from(["ivar", "feature", "reparent", "child"]).unwrap_err();
    assert!(error.to_string().contains("required"), "was: {error}");
}

#[test]
fn feature_reparent_args_convert_into_reparent_input() {
    let args = FeatureReparentArgs {
        child: Some("child".to_owned()),
        parent: "new-parent".to_owned(),
    };

    let input: crate::action::feature::reparent::ReparentInput = args.into();

    assert_eq!(input.child, "child");
    assert_eq!(input.parent, "new-parent");
}

#[test]
fn feature_integrate_args_convert_into_integrate_input() {
    let args = FeatureIntegrateArgs {
        feature: Some("child".to_owned()),
        via: Some("pr".to_owned()),
        strategy: Some("merge".to_owned()),
        name: Some("feat: add checkout tax".to_owned()),
    };

    let input: crate::action::feature::integrate::IntegrateInput = args.into();

    assert_eq!(input.feature, "child");
    assert_eq!(input.via.as_deref(), Some("pr"));
    assert_eq!(input.strategy.as_deref(), Some("merge"));
    assert_eq!(input.name.as_deref(), Some("feat: add checkout tax"));
}

#[test]
fn feature_status_args_convert_into_status_input() {
    let args = FeatureStatusArgs {
        feature: Some("parent".to_owned()),
        recursive: true,
    };

    let input: crate::action::feature::status::StatusInput = args.into();

    assert_eq!(input.feature, "parent");
    assert!(input.recursive);
}

/// The policy is fixed at creation — there is deliberately no
/// policy-configure subcommand.
#[test]
fn there_is_no_feature_policy_configure_subcommand() {
    let names: Vec<String> = Cli::command()
        .find_subcommand_mut("feature")
        .expect("the feature subcommand exists")
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_owned())
        .collect();
    for forbidden in ["configure", "policy", "set-policy"] {
        assert!(
            !names.iter().any(|name| name == forbidden),
            "a policy-configure subcommand must not exist: {names:?}"
        );
    }
    assert!(names.iter().any(|name| name == "reparent"));
}

/// Multi-occurrence `--draft`: two scoped occurrences each bind to their
/// own repo group.
#[test]
fn feature_deliver_parses_multi_repo_scoped_draft() {
    let cli = Cli::try_parse_from([
        "ivar", "feature", "deliver", "checkout", "--repo", "api", "--draft", "--repo", "web",
        "--draft",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Deliver(args)) => {
            assert_eq!(args.global_metadata.draft, None, "no global draft");
            let [api, web] = args.repo_overrides.as_slice() else {
                panic!("expected two repo groups, got {:?}", args.repo_overrides);
            };
            assert_eq!(api.repo, "api");
            assert_eq!(api.metadata.draft, Some(true));
            assert_eq!(web.repo, "web");
            assert_eq!(web.metadata.draft, Some(true));
        }
        other => panic!("expected Feature(Deliver), got {other:?}"),
    }
}

/// A `--draft` before any `--repo` is global; one after a `--repo` is scoped.
#[test]
fn feature_deliver_global_and_scoped_draft_mixed() {
    let cli = Cli::try_parse_from([
        "ivar", "feature", "deliver", "checkout", "--draft", "--repo", "api", "--draft",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Deliver(args)) => {
            // The first --draft (global position) applies to all repos not
            // covered by a scoped override.
            assert_eq!(args.global_metadata.draft, Some(true));
            // The second --draft after --repo api is scoped to api only.
            let [api] = args.repo_overrides.as_slice() else {
                panic!("expected one repo group, got {:?}", args.repo_overrides);
            };
            assert_eq!(api.repo, "api");
            assert_eq!(api.metadata.draft, Some(true));
        }
        other => panic!("expected Feature(Deliver), got {other:?}"),
    }
}

#[test]
fn session_sandbox_subcommand_is_hidden_from_session_help() {
    use clap::CommandFactory;
    let mut app = Cli::command();
    let session_subcommand = app
        .find_subcommand_mut("session")
        .expect("session subcommand must exist");
    let mut help_bytes = Vec::new();
    session_subcommand.write_help(&mut help_bytes).unwrap();
    let help_str = String::from_utf8(help_bytes).unwrap();
    assert!(
        !help_str.contains("sandbox"),
        "hidden sandbox subcommand should not appear in session help:\n{help_str}"
    );
}

#[test]
fn parse_optional_feature_positionals_omitted_and_supplied() {
    // 1. workspace
    assert!(Cli::try_parse_from(["ivar", "feature", "workspace"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "workspace", "feat-a"]).is_ok());

    // 2. integrate
    assert!(Cli::try_parse_from(["ivar", "feature", "integrate"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "integrate", "feat-a"]).is_ok());

    // 3. rename (source)
    assert!(Cli::try_parse_from(["ivar", "feature", "rename", "--name", "new-name"]).is_ok());
    assert!(
        Cli::try_parse_from(["ivar", "feature", "rename", "feat-a", "--name", "new-name"]).is_ok()
    );

    // 4. reparent (child)
    assert!(
        Cli::try_parse_from(["ivar", "feature", "reparent", "--parent", "parent-feat"]).is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "reparent",
            "child-feat",
            "--parent",
            "parent-feat"
        ])
        .is_ok()
    );

    // 5. promote (optional-before-required positional: [feature] <repo>)
    // When 1 positional is supplied, Clap assigns it to repo; feature remains None.
    let parsed =
        Cli::try_parse_from(["ivar", "feature", "promote", "my-repo"]).expect("parse 1 positional");
    if let Command::Feature(FeatureCommand::Promote(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.repo, "my-repo");
    } else {
        panic!("wrong variant");
    }
    let parsed = Cli::try_parse_from(["ivar", "feature", "promote", "feat-a", "my-repo"])
        .expect("parse 2 positionals");
    if let Command::Feature(FeatureCommand::Promote(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_owned()));
        assert_eq!(args.repo, "my-repo");
    } else {
        panic!("wrong variant");
    }

    // 6. demote (optional-before-required positional: [feature] <repo>)
    let parsed =
        Cli::try_parse_from(["ivar", "feature", "demote", "my-repo"]).expect("parse 1 positional");
    if let Command::Feature(FeatureCommand::Demote(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.repo, "my-repo");
    } else {
        panic!("wrong variant");
    }
    let parsed = Cli::try_parse_from(["ivar", "feature", "demote", "feat-a", "my-repo"])
        .expect("parse 2 positionals");
    if let Command::Feature(FeatureCommand::Demote(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_owned()));
        assert_eq!(args.repo, "my-repo");
    } else {
        panic!("wrong variant");
    }

    // 7. status
    assert!(Cli::try_parse_from(["ivar", "feature", "status"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "status", "feat-a"]).is_ok());

    // 8. deliver
    assert!(Cli::try_parse_from(["ivar", "feature", "deliver"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "deliver", "feat-a"]).is_ok());

    // 9. view
    assert!(Cli::try_parse_from(["ivar", "feature", "view"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "view", "feat-a"]).is_ok());

    // 10. execute start
    assert!(
        Cli::try_parse_from(["ivar", "feature", "execute", "start", "--plan", "plan.md"]).is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "ivar", "feature", "execute", "start", "feat-a", "--plan", "plan.md"
        ])
        .is_ok()
    );

    // 11. execute finish
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "finish",
            "--plan",
            "plan.md",
            "--report-json",
            "{}",
            "--outcome",
            "succeeded"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "finish",
            "feat-a",
            "--plan",
            "plan.md",
            "--report-json",
            "{}",
            "--outcome",
            "succeeded"
        ])
        .is_ok()
    );

    // 12. execute status
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "status"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "status", "feat-a"]).is_ok());

    // 13. execute accept-revision
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "accept-revision",
            "--plan",
            "plan.md"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "accept-revision",
            "feat-a",
            "--plan",
            "plan.md"
        ])
        .is_ok()
    );

    // 14. plan create
    assert!(Cli::try_parse_from(["ivar", "plan", "create"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "plan", "create", "feat-a"]).is_ok());

    // 15. plan show (optional-before-required positional: [feature] <artifact>)
    let parsed = Cli::try_parse_from(["ivar", "plan", "show", "plan"]).expect("parse 1 positional");
    if let Command::Plan(PlanCommand::Show(args)) = parsed.command {
        assert_eq!(args.feature, None);
    } else {
        panic!("wrong variant");
    }
    let parsed = Cli::try_parse_from(["ivar", "plan", "show", "feat-a", "plan"])
        .expect("parse 2 positionals");
    if let Command::Plan(PlanCommand::Show(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_owned()));
    } else {
        panic!("wrong variant");
    }

    // 16. plan approve (optional-before-required positional: [feature] <gate>)
    let parsed = Cli::try_parse_from(["ivar", "plan", "approve", "requirements"])
        .expect("parse 1 positional");
    if let Command::Plan(PlanCommand::Approve(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.gate, "requirements");
    } else {
        panic!("wrong variant");
    }
    let parsed = Cli::try_parse_from(["ivar", "plan", "approve", "feat-a", "requirements"])
        .expect("parse 2 positionals");
    if let Command::Plan(PlanCommand::Approve(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_owned()));
        assert_eq!(args.gate, "requirements");
    } else {
        panic!("wrong variant");
    }

    // 17. plan invalidate (optional-before-required positional: [feature] <gate>)
    let parsed = Cli::try_parse_from(["ivar", "plan", "invalidate", "analysis"])
        .expect("parse 1 positional");
    if let Command::Plan(PlanCommand::Invalidate(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.gate, "analysis");
    } else {
        panic!("wrong variant");
    }
    let parsed = Cli::try_parse_from(["ivar", "plan", "invalidate", "feat-a", "analysis"])
        .expect("parse 2 positionals");
    if let Command::Plan(PlanCommand::Invalidate(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_owned()));
        assert_eq!(args.gate, "analysis");
    } else {
        panic!("wrong variant");
    }

    // 18. close
    assert!(Cli::try_parse_from(["ivar", "feature", "close", "--outcome", "delivered"]).is_ok());
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "close",
            "feat-a",
            "--outcome",
            "delivered"
        ])
        .is_ok()
    );

    // 19. delete
    assert!(Cli::try_parse_from(["ivar", "feature", "delete"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "delete", "feat-a"]).is_ok());

    // 20. cleanup
    assert!(Cli::try_parse_from(["ivar", "feature", "cleanup"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "cleanup", "feat-a"]).is_ok());

    // 21. rebase
    assert!(Cli::try_parse_from(["ivar", "feature", "rebase"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "rebase", "feat-a"]).is_ok());
}

#[test]
fn feature_create_remains_required_positional() {
    assert!(Cli::try_parse_from(["ivar", "feature", "create"]).is_err());
    assert!(Cli::try_parse_from(["ivar", "feature", "create", "feat-a"]).is_ok());
}

#[test]
fn repo_create_requires_exactly_one_mode_and_maps_public() {
    assert!(Cli::try_parse_from(["ivar", "repo", "create", "notes"]).is_err());
    assert!(
        Cli::try_parse_from(["ivar", "repo", "create", "notes", "--local", "--remote"]).is_err()
    );
    assert!(
        Cli::try_parse_from(["ivar", "repo", "create", "notes", "--local", "--public"]).is_err()
    );

    let cli =
        Cli::try_parse_from(["ivar", "repo", "create", "notes", "--remote", "--public"]).unwrap();
    let Command::Repo(RepoCommand::Create(args)) = cli.command else {
        panic!("expected repo create")
    };
    let input: repo_create::CreateInput = args.into();
    assert_eq!(input.mode, repo_create::CreateMode::Remote { public: true });
    assert_eq!(input.default_branch, None);

    let cli_local = Cli::try_parse_from(["ivar", "repo", "create", "notes", "--local"]).unwrap();
    let Command::Repo(RepoCommand::Create(args_local)) = cli_local.command else {
        panic!("expected repo create")
    };
    let input_local: repo_create::CreateInput = args_local.into();
    assert_eq!(input_local.mode, repo_create::CreateMode::Local);
    assert_eq!(input_local.default_branch, None);
}

#[test]
fn execute_finish_print_schema_needs_no_report_or_outcome() {
    match Cli::try_parse_from(["ivar", "feature", "execute", "finish", "--print-schema"])
        .unwrap()
        .command
    {
        Command::Feature(FeatureCommand::Execute(ExecuteCommand::Finish(args))) => {
            assert!(args.print_schema);
            assert_eq!(args.report_json, None);
            assert_eq!(args.outcome, None);
        }
        other => panic!("expected execute finish, got {other:?}"),
    }
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "finish",
            "--outcome",
            "succeeded"
        ])
        .is_err(),
        "without --print-schema, --report-json stays required"
    );
}

#[test]
fn execute_finish_help_points_to_print_schema() {
    let mut cli = Cli::command();
    let finish = cli
        .find_subcommand_mut("feature")
        .unwrap()
        .find_subcommand_mut("execute")
        .unwrap()
        .find_subcommand_mut("finish")
        .unwrap();
    let report_help = finish
        .get_arguments()
        .find(|arg| arg.get_id() == "report_json")
        .and_then(|arg| arg.get_help())
        .unwrap()
        .to_string();
    assert!(report_help.contains("--print-schema"), "{report_help}");
    assert!(
        finish
            .render_long_help()
            .to_string()
            .contains("--print-schema")
    );
}

#[test]
fn execute_checkpoint_requires_a_positive_wave_and_a_summary() {
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "checkpoint",
            "checkout",
            "--wave",
            "2",
            "--summary",
            "wave 2 approved",
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from(["ivar", "feature", "execute", "checkpoint", "--wave", "1"]).is_err()
    );
    assert!(
        Cli::try_parse_from([
            "ivar",
            "feature",
            "execute",
            "checkpoint",
            "--wave",
            "0",
            "--summary",
            "x",
        ])
        .is_err()
    );
}

#[test]
fn repo_view_defaults_to_every_declared_repo_and_accepts_a_subset() {
    let cli = Cli::try_parse_from(["ivar", "repo", "view"]).unwrap();
    let Command::Repo(RepoCommand::View(args)) = cli.command else {
        panic!("expected repo view")
    };
    let input: repo_view::ViewInput = args.into();
    assert!(
        input.repos.is_empty(),
        "naming no repo means every declared repo, not none"
    );

    let cli = Cli::try_parse_from(["ivar", "repo", "view", "api", "web"]).unwrap();
    let Command::Repo(RepoCommand::View(args)) = cli.command else {
        panic!("expected repo view")
    };
    let input: repo_view::ViewInput = args.into();
    assert_eq!(input.repos, vec!["api".to_owned(), "web".to_owned()]);
}

#[test]
fn feature_deliver_parses_repeated_only_independently_of_repo_groups() {
    let cli = Cli::try_parse_from([
        "ivar", "feature", "deliver", "checkout", "--only", "api", "--name", "global", "--only",
        "web",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Deliver(args)) => {
            let input: deliver::DeliverInput = args.into();
            assert_eq!(input.only, vec!["api".to_owned(), "web".to_owned()]);
            assert_eq!(input.global_metadata.title.as_deref(), Some("global"));
            assert!(input.repo_overrides.is_empty());
        }
        other => panic!("expected feature deliver, got {other:?}"),
    }
}

#[test]
fn upgrade_parses_with_and_without_check() {
    let cli = Cli::try_parse_from(["ivar", "upgrade"]).unwrap();
    match cli.command {
        Command::Upgrade(args) => assert!(!UpgradeInput::from(args).check),
        other => panic!("expected Upgrade, got {other:?}"),
    }

    let cli = Cli::try_parse_from(["ivar", "upgrade", "--check"]).unwrap();
    match cli.command {
        Command::Upgrade(args) => assert!(UpgradeInput::from(args).check),
        other => panic!("expected Upgrade, got {other:?}"),
    }
}

#[test]
fn guard_takes_a_hidden_slice_for_claude_code_only() {
    let cli = Cli::try_parse_from(["ivar", "guard", "--provider", "claude-code", "--slice", "2"])
        .unwrap();
    let Command::Guard(args) = cli.command else {
        panic!("expected the guard command");
    };
    let input = guard_cmd::GuardInput::try_from(args).unwrap();
    // `cli` may import only `action` (tests/static_checks/architecture.rs), so compare by Debug name.
    assert_eq!(format!("{:?}", input.provider), "ClaudeCode");
    assert_eq!(input.slice, Some(2));

    let cli = Cli::try_parse_from(["ivar", "guard", "--provider", "omp", "--slice", "1"]).unwrap();
    let Command::Guard(args) = cli.command else {
        panic!("expected the guard command");
    };
    assert!(guard_cmd::GuardInput::try_from(args).is_err());

    let cli = Cli::try_parse_from(["ivar", "guard", "--provider", "omp"]).unwrap();
    let Command::Guard(args) = cli.command else {
        panic!("expected the guard command");
    };
    assert_eq!(guard_cmd::GuardInput::try_from(args).unwrap().slice, None);

    let mut root = Cli::command();
    let help = root
        .find_subcommand_mut("guard")
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(!help.contains("--slice"), "{help}");
}

/// The long help a non-TTY `ivar feature deliver --help` prints: clap wraps
/// at 100 columns when it cannot read a terminal width.
fn deliver_long_help() -> String {
    Cli::command()
        .find_subcommand("feature")
        .unwrap()
        .find_subcommand("deliver")
        .unwrap()
        .clone()
        .term_width(100)
        .render_long_help()
        .to_string()
}

/// `feature deliver --help` documents `--draft`, its positional repository
/// scope (global before `--repo`, scoped after `--repo <name>`), and its
/// conflict with `--land`.
#[test]
fn draft_help_documents_scope_and_land_conflict() {
    let stdout = deliver_long_help();
    assert!(
        stdout.contains("--draft"),
        "--draft flag must appear in deliver help: {stdout}"
    );
    assert!(
        stdout.contains("--repo"),
        "--repo flag must appear in deliver help: {stdout}"
    );
    assert!(
        stdout.contains("--land"),
        "--land flag must appear in deliver help: {stdout}"
    );

    let draft_section = find_flag_help(&stdout, "draft");
    assert!(
        draft_section.contains("global") || draft_section.contains("before"),
        "--draft help must describe positional scoping (global before --repo): {draft_section}"
    );
    assert!(
        draft_section.contains("--repo"),
        "--draft help must mention --repo scoping: {draft_section}"
    );
    assert!(
        draft_section.contains("land") || draft_section.contains("--land"),
        "--draft help must mention the --land conflict: {draft_section}"
    );
}

#[test]
fn deliver_help_states_name_and_body_are_fingerprinted() {
    let stdout = deliver_long_help();

    for flag in ["name", "body", "fingerprint"] {
        let section = find_flag_help(&stdout, flag);
        assert!(
            section.contains("fingerprint"),
            "--{flag} help must say it is part of the fingerprint: {section}"
        );
    }
}

/// Extract the help text for a specific long flag from `--help` output.
/// Returns the description line(s) for `--<flag_name>`.
fn find_flag_help(help_output: &str, flag_name: &str) -> String {
    let marker = format!("--{flag_name}");
    let lines: Vec<&str> = help_output.lines().collect();
    let mut result = String::new();
    let mut collecting = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with(&marker) {
            collecting = true;
            result.push_str(trimmed);
            result.push('\n');
            continue;
        }
        if collecting {
            // Stop at the next flag definition or empty section
            if trimmed.starts_with("--") || trimmed.is_empty() {
                break;
            }
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    result
}
