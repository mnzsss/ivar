//! The command reference in `docs/reference/commands.md` is generated from
//! `clap`, and this test is what keeps it that way. It also proves that every
//! relative link in the documentation corpus resolves on disk.
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
//! A broken relative link anywhere in `docs/`, `README.md`, or `ARCHITECTURE.md`
//! is the other half of documentation drifting from the repository, and fails
//! here too.
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

/// Every Markdown file whose links this test checks: the two root documents
/// and everything under `docs/`.
fn markdown_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = vec![root.join("README.md"), root.join("ARCHITECTURE.md")];
    let mut stack = vec![root.join("docs")];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Pull every link and image target out of one file's Markdown.
///
/// Two forms carry a target: `](…)` covers inline links and images alike,
/// and `<img src="…">` covers the one HTML tag in the corpus
/// (`README.md:4`). Reference-style definitions are absent from these files
/// and are not handled — if one is ever added, its link will simply go
/// unchecked rather than misreported.
fn link_targets(text: &str) -> Vec<String> {
    let mut found = Vec::new();

    let mut rest = text;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else { break };
        let target = after[..end].trim();
        // `[text](<url> "title")` — the title is not part of the path.
        let target = target.split_whitespace().next().unwrap_or(target);
        if !target.is_empty() {
            found.push(target.to_owned());
        }
        rest = &after[end + 1..];
    }

    let mut rest = text;
    while let Some(start) = rest.find("<img src=\"") {
        let after = &rest[start + 10..];
        let Some(end) = after.find('"') else { break };
        found.push(after[..end].to_owned());
        rest = &after[end + 1..];
    }

    found
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

/// A relative link in the documentation must point at a file that exists.
///
/// The corpus is small and entirely relative: 41 relative targets across 19
/// files, all resolving today. This keeps it that way — a renamed doc that
/// leaves a dangling link fails here rather than in a reader's browser.
#[test]
fn every_relative_documentation_link_resolves() {
    let mut broken = Vec::new();

    for file in markdown_files() {
        let text = std::fs::read_to_string(&file).unwrap();
        let dir = file.parent().unwrap();

        for target in link_targets(&text) {
            // An absolute URL, a mail link and a same-file anchor are not
            // filesystem paths and cannot be checked by looking on disk.
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
                || target.starts_with('#')
            {
                continue;
            }

            // `guides/day-to-day.md#nested-subfeatures` names a file and a
            // heading inside it. Only the file half is on disk.
            let path = target.split('#').next().unwrap_or(&target);
            if path.is_empty() {
                continue;
            }

            if !dir.join(path).exists() {
                broken.push(format!("{} -> {target}", file.display()));
            }
        }
    }

    assert!(
        broken.is_empty(),
        "documentation links that resolve to nothing ({}):\n  {}",
        broken.len(),
        broken.join("\n  ")
    );
}

/// A broken extractor would make the assertion above vacuous, so the count
/// is asserted too. 51 links and images exist today; the floor is loose on
/// purpose, to catch "matched nothing" rather than to pin a number.
#[test]
fn documentation_contains_links_to_check() {
    let found: usize = markdown_files()
        .iter()
        .map(|file| link_targets(&std::fs::read_to_string(file).unwrap()).len())
        .sum();

    assert!(
        found >= 40,
        "expected at least 40 link targets, found {found} — the extractor is broken"
    );
}
