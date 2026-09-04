//! The command reference in `docs/reference/commands.md` is generated from
//! `clap`, and this test is what keeps it that way.
//!
//! # Why a test and not a build script
//!
//! The decision on record (the docs-structure ticket) is a **hybrid** reference:
//! flags, arguments and types generated so they cannot drift from the binary,
//! surrounded by hand-written prose about when to reach for a verb. A build
//! script would regenerate silently on every build, which makes a generated file
//! that is committed to git churn without anyone deciding to. A test states the
//! rule the other way round: the committed file is the artifact, and CI fails
//! the moment the binary and the file disagree.
//!
//! So the generated half lives in `commands.md` between two markers, and
//! everything outside them is prose this test never touches.
//!
//! To regenerate after changing the CLI:
//!
//! ```sh
//! IVAR_UPDATE_DOCS=1 cargo test --test docs_reference
//! ```
//!
//! `clap-markdown` is deliberately not used. It renders a whole document with
//! its own headings and ordering, which cannot be interleaved with prose per
//! command — the exact shape the decision asked for — and it is a dependency for
//! roughly the amount of code below.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Command, CommandFactory};
use ivar::cli::root::Cli;

/// Everything between these two lines is generated. Prose lives outside them.
const BEGIN: &str = "<!-- BEGIN GENERATED COMMANDS -->";
const END: &str = "<!-- END GENERATED COMMANDS -->";

fn reference_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference/commands.md")
}

/// Render the whole command tree, depth-first, in declaration order — the same
/// order `--help` lists them, so the page and the binary agree on more than
/// just content.
fn render(command: &mut Command) -> String {
    let mut out = String::new();
    out.push_str(
        "<!-- Generated from clap by tests/docs_reference.rs. Do not edit by hand: run\n     \
         `IVAR_UPDATE_DOCS=1 cargo test --test docs_reference`. -->\n",
    );
    let name = command.get_name().to_owned();
    render_command(&mut out, command, &name, 3);
    out
}

fn render_command(out: &mut String, command: &mut Command, path: &str, depth: usize) {
    let heading = "#".repeat(depth.min(6));
    let _ = writeln!(out, "\n{heading} `{path}`\n");

    if let Some(about) = command
        .get_long_about()
        .or_else(|| command.get_about())
        .map(ToString::to_string)
    {
        // clap's long_about carries the doc comment's own line breaks; collapse
        // them so the table below is not pushed apart by hard wraps.
        let _ = writeln!(out, "{}\n", collapse(&about));
    }

    let positionals: Vec<_> = command.get_positionals().collect();
    let options: Vec<_> = command
        .get_arguments()
        .filter(|arg| !arg.is_positional())
        // `--json` and `--color` are global and documented once, at the root.
        .filter(|arg| depth == 3 || !arg.is_global_set())
        .filter(|arg| !arg.is_hide_set())
        .collect();

    if !positionals.is_empty() {
        let _ = writeln!(out, "| argument | required | description |");
        let _ = writeln!(out, "| --- | --- | --- |");
        for arg in &positionals {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} |",
                arg.get_id(),
                if arg.is_required_set() { "yes" } else { "no" },
                describe(arg)
            );
        }
        out.push('\n');
    }

    if !options.is_empty() {
        let _ = writeln!(out, "| flag | value | default | description |");
        let _ = writeln!(out, "| --- | --- | --- | --- |");
        for arg in &options {
            let long = arg
                .get_long()
                .map(|long| format!("`--{long}`"))
                .unwrap_or_default();
            let short = arg
                .get_short()
                .map(|short| format!(" / `-{short}`"))
                .unwrap_or_default();
            // A boolean flag has no value, but clap still reports a value *name*
            // for it (the id, uppercased). Rendering that would document
            // `--json <JSON>`, which is not accepted.
            let value = arg
                .get_value_names()
                .filter(|_| arg.get_action().takes_values())
                .map(|names| {
                    names
                        .iter()
                        .map(|name| format!("`<{name}>`"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let default = arg
                .get_default_values()
                .iter()
                .map(|value| format!("`{}`", value.to_string_lossy()))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "| {long}{short} | {value} | {default} | {} |",
                describe(arg)
            );
        }
        out.push('\n');
    }

    let subcommand_names: Vec<String> = command
        .get_subcommands()
        .filter(|sub| sub.get_name() != "help")
        // A hidden command is absent from `--help`, and this page documents the
        // public surface. `git-credential` is the case: git invokes it through
        // the credential-helper protocol, and a reader has no use for it.
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| sub.get_name().to_owned())
        .collect();

    for name in subcommand_names {
        let child_path = format!("{path} {name}");
        let child = command
            .get_subcommands_mut()
            .find(|sub| sub.get_name() == name)
            .expect("subcommand just enumerated");
        render_command(out, child, &child_path, depth + 1);
    }
}

fn describe(arg: &clap::Arg) -> String {
    arg.get_long_help()
        .or_else(|| arg.get_help())
        .map(|help| collapse(&help.to_string()))
        .unwrap_or_default()
}

/// Fold a doc comment into one table-safe line: newlines become spaces, and a
/// literal `|` would otherwise end the cell.
fn collapse(text: &str) -> String {
    text.replace('|', "\\|")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_command_reference_matches_the_binary() {
    let path = reference_path();
    let generated = render(&mut Cli::command());

    let existing = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} is missing ({error}). Create it with the two markers, then run \
             `IVAR_UPDATE_DOCS=1 cargo test --test docs_reference`.",
            path.display()
        )
    });

    let (before, rest) = existing.split_once(BEGIN).unwrap_or_else(|| {
        panic!(
            "{} has no `{BEGIN}` marker — the generated block needs somewhere to go",
            path.display()
        )
    });
    let (_, after) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("{} has `{BEGIN}` but no `{END}`", path.display()));

    let rebuilt = format!("{before}{BEGIN}\n{generated}{END}{after}");

    if rebuilt == existing {
        return;
    }

    if std::env::var_os("IVAR_UPDATE_DOCS").is_some() {
        std::fs::write(&path, &rebuilt).unwrap();
        return;
    }

    panic!(
        "{} is out of date with the CLI.\n\nRegenerate it:\n    \
         IVAR_UPDATE_DOCS=1 cargo test --test docs_reference\n",
        path.display()
    );
}

#[test]
fn documented_provider_set_equals_all_providers() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let arch_path = manifest_dir.join("ARCHITECTURE.md");
    let content = std::fs::read_to_string(&arch_path).unwrap();

    // Ensure ARCHITECTURE.md environment table lists all three providers
    assert!(
        content.contains("`claude-code`, `opencode`, or `omp`"),
        "ARCHITECTURE.md must document all three providers in IVAR_PROVIDER description"
    );

    // Verify Provider::ALL coverage
    let providers = ivar::domain::provider::Provider::ALL;
    assert_eq!(
        providers.len(),
        3,
        "Provider::ALL must contain exactly 3 providers"
    );
    assert!(providers.contains(&ivar::domain::provider::Provider::ClaudeCode));
    assert!(providers.contains(&ivar::domain::provider::Provider::OpenCode));
    assert!(providers.contains(&ivar::domain::provider::Provider::Omp));
}
