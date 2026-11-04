#![allow(clippy::unwrap_used)]

use clap::Parser;

use crate::cli::memory::MemoryCommand;
use crate::cli::root::{Cli, Command};

#[test]
fn memory_status_parses_with_json() {
    let cli = Cli::try_parse_from(["ivar", "memory", "status", "--json"]).unwrap();

    assert!(matches!(
        cli.command,
        Command::Memory(MemoryCommand::Status)
    ));
    assert!(cli.json);
}

#[test]
fn memory_clean_parses() {
    let cli = Cli::try_parse_from(["ivar", "memory", "clean"]).unwrap();

    assert!(matches!(cli.command, Command::Memory(MemoryCommand::Clean)));
}
