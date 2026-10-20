#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::Parser;

use crate::cli::agent::AgentCommand;
use crate::cli::root::{Cli, Command};

#[test]
fn the_timer_invocation_parses_with_the_hidden_trigger() {
    let cli =
        Cli::try_parse_from(["ivar", "agent", "run", "nightly", "--trigger", "schedule"]).unwrap();
    let Command::Agent(AgentCommand::Run(args)) = cli.command else {
        panic!("not agent run")
    };
    assert_eq!(
        (args.name.as_str(), args.trigger.as_str()),
        ("nightly", "schedule")
    );
}

#[test]
fn a_manual_run_defaults_its_trigger() {
    let cli = Cli::try_parse_from(["ivar", "agent", "run", "nightly"]).unwrap();
    let Command::Agent(AgentCommand::Run(args)) = cli.command else {
        panic!("not agent run")
    };
    assert_eq!(args.trigger, "manual");
}

#[test]
fn feedback_accepts_only_up_or_down() {
    assert!(Cli::try_parse_from(["ivar", "agent", "feedback", "r1", "--rating", "meh"]).is_err());
    assert!(
        Cli::try_parse_from([
            "ivar",
            "agent",
            "feedback",
            "r1",
            "--rating",
            "up",
            "--comment",
            "ok"
        ])
        .is_ok()
    );
}

#[test]
fn the_trigger_flag_is_hidden_from_help() {
    let mut command = <Cli as clap::CommandFactory>::command();
    let help = command
        .find_subcommand_mut("agent")
        .unwrap()
        .find_subcommand_mut("run")
        .unwrap()
        .render_help()
        .to_string();
    assert!(!help.contains("--trigger"), "{help}");
}

#[test]
fn the_hidden_sandbox_launcher_takes_write_roots_and_a_command() {
    let cli = Cli::try_parse_from([
        "ivar", "agent", "sandbox", "--write", "/a", "--write", "/b", "--", "claude", "-p", "Go.",
    ])
    .unwrap();
    let Command::Agent(AgentCommand::Sandbox(args)) = cli.command else {
        panic!("not agent sandbox")
    };
    assert_eq!(args.write, ["/a", "/b"]);
    assert_eq!(args.command, ["claude", "-p", "Go."]);
}
