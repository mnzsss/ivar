#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::CommandFactory as _;

use super::*;

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
        "figma-gaio",
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
            assert_eq!(args.child, "child");
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
        child: "child".to_owned(),
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
            assert_eq!(args.feature, "child");
            assert_eq!(args.via.as_deref(), Some("pr"));
            assert_eq!(args.strategy.as_deref(), Some("rebase"));
        }
        other => panic!("expected Feature(Integrate), got {other:?}"),
    }
}

#[test]
fn feature_integrate_args_convert_into_integrate_input() {
    let args = FeatureIntegrateArgs {
        feature: "child".to_owned(),
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
        feature: "parent".to_owned(),
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
