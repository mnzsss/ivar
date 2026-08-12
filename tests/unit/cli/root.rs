#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::{CommandFactory as _, Parser as _};

use super::*;

#[test]
fn cli_definition_is_valid() {
    // `debug_assert` panics on a malformed clap definition (duplicate
    // ids, conflicting args, ...) — the cheapest test that the derive
    // actually produced a usable `Command`.
    Cli::command().debug_assert();
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
