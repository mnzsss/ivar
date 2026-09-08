#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::CommandFactory as _;

use super::*;

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

/// git appends the operation it wants to the helper command line: the
/// registered `!ivar git-credential` is invoked as `ivar git-credential get`,
/// `… store`, `… erase`. A definition that takes no operand makes clap refuse
/// every one of them, and the refusal lands in the middle of a `git push`.
#[test]
fn git_credential_accepts_the_operation_git_appends() {
    for operation in ["get", "store", "erase"] {
        let cli = Cli::try_parse_from(["ivar", "git-credential", operation])
            .unwrap_or_else(|error| panic!("git-credential {operation} refused: {error}"));

        match cli.command {
            Command::GitCredential(args) => {
                assert_eq!(args.operation.as_deref(), Some(operation));
            }
            other => panic!("expected GitCredential, got {other:?}"),
        }
    }
}

/// `--base` names the branch new promotions should start from; omitted, it
/// stays `None` and each repo's own default branch stands in.
#[test]
fn feature_create_accepts_base() {
    let cli = Cli::try_parse_from(["ivar", "feature", "create", "checkout", "--base", "develop"])
        .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Create(args)) => {
            assert_eq!(args.base.as_deref(), Some("develop"));
        }
        other => panic!("expected Feature(Create), got {other:?}"),
    }
}

/// A subfeature is created with `--parent`, which derives the base from the
/// parent's branch; `--via`/`--strategy` persist the feature's own policy
/// override.
#[test]
fn feature_create_accepts_parent_via_and_strategy() {
    let cli = Cli::try_parse_from([
        "ivar",
        "feature",
        "create",
        "child",
        "--parent",
        "parent",
        "--via",
        "pr",
        "--strategy",
        "rebase",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Create(args)) => {
            assert_eq!(args.parent.as_deref(), Some("parent"));
            assert_eq!(args.via.as_deref(), Some("pr"));
            assert_eq!(args.strategy.as_deref(), Some("rebase"));
        }
        other => panic!("expected Feature(Create), got {other:?}"),
    }
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

#[test]
fn feature_reparent_parses_a_child_and_parent() {
    let cli = Cli::try_parse_from([
        "ivar",
        "feature",
        "reparent",
        "child",
        "--parent",
        "new-parent",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Reparent(args)) => {
            assert_eq!(args.child.as_deref(), Some("child"));
            assert_eq!(args.parent, "new-parent");
        }
        other => panic!("expected Feature(Reparent), got {other:?}"),
    }
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
fn feature_status_accepts_recursive() {
    let cli = Cli::try_parse_from(["ivar", "feature", "status", "parent", "--recursive"]).unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Status(args)) => {
            assert!(args.recursive);
        }
        other => panic!("expected Feature(Status), got {other:?}"),
    }
}

#[test]
fn feature_integrate_accepts_via_and_strategy() {
    let cli = Cli::try_parse_from([
        "ivar",
        "feature",
        "integrate",
        "child",
        "--via",
        "pr",
        "--strategy",
        "rebase",
    ])
    .unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Integrate(args)) => {
            assert_eq!(args.feature.as_deref(), Some("child"));
            assert_eq!(args.via.as_deref(), Some("pr"));
            assert_eq!(args.strategy.as_deref(), Some("rebase"));
        }
        other => panic!("expected Feature(Integrate), got {other:?}"),
    }
}

#[test]
fn feature_integrate_args_convert_into_integrate_input() {
    let args = FeatureIntegrateArgs {
        feature: Some("child".to_owned()),
        via: Some("pr".to_owned()),
        strategy: Some("merge".to_owned()),
    };

    let input: crate::action::feature::integrate::IntegrateInput = args.into();

    assert_eq!(input.feature, "child");
    assert_eq!(input.via.as_deref(), Some("pr"));
    assert_eq!(input.strategy.as_deref(), Some("merge"));
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

/// `--onto` collapses the base for every promoted repo — see
/// `action::feature::rebase`.
#[test]
fn feature_rebase_accepts_onto() {
    let cli =
        Cli::try_parse_from(["ivar", "feature", "rebase", "checkout", "--onto", "main"]).unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Rebase(args)) => {
            assert_eq!(args.onto.as_deref(), Some("main"));
        }
        other => panic!("expected Feature(Rebase), got {other:?}"),
    }
}

#[test]
fn feature_deliver_parses_draft_intent() {
    let cli = Cli::try_parse_from(["ivar", "feature", "deliver", "checkout", "--draft"]).unwrap();

    match cli.command {
        Command::Feature(FeatureCommand::Deliver(args)) => {
            assert_eq!(args.global_metadata.draft, Some(true));
        }
        other => panic!("expected Feature(Deliver), got {other:?}"),
    }
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

/// A future git may name an operation this build has never heard of. Parsing
/// must still succeed — gitcredentials(7) requires the helper to ignore what
/// it does not implement, and it cannot ignore what clap rejected first.
#[test]
fn git_credential_accepts_an_operation_it_does_not_implement() {
    let cli = Cli::try_parse_from(["ivar", "git-credential", "capability"])
        .expect("an unknown operation parses; the helper ignores it");

    match cli.command {
        Command::GitCredential(args) => assert_eq!(args.operation.as_deref(), Some("capability")),
        other => panic!("expected GitCredential, got {other:?}"),
    }
}

#[test]
fn session_sandbox_parses_hidden_subcommand_with_trailing_argv() {
    let cli = Cli::try_parse_from([
        "ivar",
        "session",
        "sandbox",
        "--session",
        "6f0c9d5f-0000-4000-8000-000000000000",
        "--",
        "claude",
        "--resume",
    ])
    .expect("hidden session sandbox subcommand must parse");

    match cli.command {
        Command::Session(SessionCommand::Sandbox(args)) => {
            assert_eq!(args.session, "6f0c9d5f-0000-4000-8000-000000000000");
            assert_eq!(args.command, vec!["claude", "--resume"]);
        }
        other => panic!("expected SessionCommand::Sandbox, got {other:?}"),
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
    assert!(Cli::try_parse_from(["ivar", "feature", "rename", "feat-a", "--name", "new-name"]).is_ok());

    // 4. reparent (child)
    assert!(Cli::try_parse_from(["ivar", "feature", "reparent", "--parent", "parent-feat"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "reparent", "child-feat", "--parent", "parent-feat"]).is_ok());

    // 5. promote (optional-before-required positional: [feature] <repo>)
    // When 1 positional is supplied, Clap assigns it to repo; feature remains None.
    let parsed = Cli::try_parse_from(["ivar", "feature", "promote", "my-repo"]).expect("parse 1 positional");
    if let Command::Feature(FeatureCommand::Promote(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.repo, "my-repo");
    } else { panic!("wrong variant"); }
    let parsed = Cli::try_parse_from(["ivar", "feature", "promote", "feat-a", "my-repo"]).expect("parse 2 positionals");
    if let Command::Feature(FeatureCommand::Promote(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_string()));
        assert_eq!(args.repo, "my-repo");
    } else { panic!("wrong variant"); }

    // 6. demote (optional-before-required positional: [feature] <repo>)
    let parsed = Cli::try_parse_from(["ivar", "feature", "demote", "my-repo"]).expect("parse 1 positional");
    if let Command::Feature(FeatureCommand::Demote(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.repo, "my-repo");
    } else { panic!("wrong variant"); }
    let parsed = Cli::try_parse_from(["ivar", "feature", "demote", "feat-a", "my-repo"]).expect("parse 2 positionals");
    if let Command::Feature(FeatureCommand::Demote(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_string()));
        assert_eq!(args.repo, "my-repo");
    } else { panic!("wrong variant"); }

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
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "start", "--plan", "plan.md"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "start", "feat-a", "--plan", "plan.md"]).is_ok());

    // 11. execute finish
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "finish", "--plan", "plan.md", "--report-json", "{}", "--outcome", "succeeded"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "finish", "feat-a", "--plan", "plan.md", "--report-json", "{}", "--outcome", "succeeded"]).is_ok());

    // 12. execute status
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "status"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "status", "feat-a"]).is_ok());

    // 13. execute accept-revision
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "accept-revision", "--plan", "plan.md"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "execute", "accept-revision", "feat-a", "--plan", "plan.md"]).is_ok());

    // 14. plan create
    assert!(Cli::try_parse_from(["ivar", "plan", "create"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "plan", "create", "feat-a"]).is_ok());

    // 15. plan show (optional-before-required positional: [feature] <artifact>)
    let parsed = Cli::try_parse_from(["ivar", "plan", "show", "plan"]).expect("parse 1 positional");
    if let Command::Plan(PlanCommand::Show(args)) = parsed.command {
        assert_eq!(args.feature, None);
    } else { panic!("wrong variant"); }
    let parsed = Cli::try_parse_from(["ivar", "plan", "show", "feat-a", "plan"]).expect("parse 2 positionals");
    if let Command::Plan(PlanCommand::Show(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_string()));
    } else { panic!("wrong variant"); }

    // 16. plan approve (optional-before-required positional: [feature] <gate>)
    let parsed = Cli::try_parse_from(["ivar", "plan", "approve", "requirements"]).expect("parse 1 positional");
    if let Command::Plan(PlanCommand::Approve(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.gate, "requirements");
    } else { panic!("wrong variant"); }
    let parsed = Cli::try_parse_from(["ivar", "plan", "approve", "feat-a", "requirements"]).expect("parse 2 positionals");
    if let Command::Plan(PlanCommand::Approve(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_string()));
        assert_eq!(args.gate, "requirements");
    } else { panic!("wrong variant"); }

    // 17. plan invalidate (optional-before-required positional: [feature] <gate>)
    let parsed = Cli::try_parse_from(["ivar", "plan", "invalidate", "analysis"]).expect("parse 1 positional");
    if let Command::Plan(PlanCommand::Invalidate(args)) = parsed.command {
        assert_eq!(args.feature, None);
        assert_eq!(args.gate, "analysis");
    } else { panic!("wrong variant"); }
    let parsed = Cli::try_parse_from(["ivar", "plan", "invalidate", "feat-a", "analysis"]).expect("parse 2 positionals");
    if let Command::Plan(PlanCommand::Invalidate(args)) = parsed.command {
        assert_eq!(args.feature, Some("feat-a".to_string()));
        assert_eq!(args.gate, "analysis");
    } else { panic!("wrong variant"); }

    // 18. close
    assert!(Cli::try_parse_from(["ivar", "feature", "close", "--outcome", "delivered"]).is_ok());
    assert!(Cli::try_parse_from(["ivar", "feature", "close", "feat-a", "--outcome", "delivered"]).is_ok());

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
fn session_relay_remains_required_positional() {
    assert!(Cli::try_parse_from(["ivar", "session", "relay", "--provider", "claude-code"]).is_err());
    assert!(Cli::try_parse_from(["ivar", "session", "relay", "feat-a", "--provider", "claude-code"]).is_ok());
}

#[test]
fn feature_create_remains_required_positional() {
    assert!(Cli::try_parse_from(["ivar", "feature", "create"]).is_err());
    assert!(Cli::try_parse_from(["ivar", "feature", "create", "feat-a"]).is_ok());
}
