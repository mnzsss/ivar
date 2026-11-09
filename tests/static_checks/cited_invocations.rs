//! Every `ivar ...` invocation quoted in shipped prose and repository
//! documentation must parse against the real CLI.
//!
//! `docs_reference` keeps the *generated* half of the docs honest by
//! rendering `clap` and failing on disagreement. This is the same rule for the
//! hand-written half: prose cites invocations, and `clap` is the only
//! authority on whether one is real.
//!
//! Existence is not the interesting part — every subcommand cited today
//! exists. Arity is: `ivar session convert <a> <b>` names a real subcommand
//! and is still wrong, because `convert` takes one positional. Feeding the
//! whole line to `clap` is what catches that. Citing a group without a verb
//! (`` `ivar repo` ``) is how a heading names a group; clap answers that with
//! help rather than a rejection, which the not-drift set forgives.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use clap::CommandFactory;
use ivar::cli::root::Cli;
use ivar::domain::name::{HallName, RepoName};
use ivar::harness::commands::catalog;
use ivar::harness::config::instructions::build_block;
/// A quoted invocation and where it came from.
struct Citation {
    source: String,
    text: String,
}

/// Pull every `` `ivar ...` `` span out of `text`.
///
/// Matches a backtick, the literal `ivar`, then a space — so `ivar.json`
/// and `ivar-review.md` never match — up to the closing backtick. A span
/// with a newline in it is a wrapped sentence, not an invocation, and is
/// skipped.
fn citations(source: &str, text: &str) -> Vec<Citation> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("`ivar ") {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else { break };
        let span = &after[..end];
        if !span.contains('\n') && !is_placeholder_only(span) {
            found.push(Citation {
                source: source.to_owned(),
                text: span.to_owned(),
            });
        }
        rest = &after[end + 1..];
    }
    found
}

/// Split an invocation into argv, replacing placeholder tokens with a
/// dummy value and dropping prose notation.
///
/// `<feature>`, `$ARGUMENTS` and `"$IVAR_FEATURE"` stand for values the
/// caller supplies; the test only asks whether the *shape* parses, so each
/// becomes `x`. A literal like `requirements` in
/// `plan approve <feature> requirements` is a value-enum variant and must
/// survive untouched.
///
/// A `[...]` span is prose notation for "optional", not argv: nobody types
/// the brackets. `discovery create <name> [--title <title>]` documents one
/// required positional and one optional flag, so the span is dropped and
/// the required shape is what gets parsed.
fn argv(invocation: &str) -> Vec<String> {
    let mut in_optional = false;
    invocation
        .split_whitespace()
        .filter_map(|token| {
            if in_optional {
                in_optional = !token.ends_with(']');
                return None;
            }
            if token.starts_with('[') {
                in_optional = !token.ends_with(']');
                return None;
            }
            if token == "…" {
                return None;
            }
            let bare = token.trim_matches('"');
            Some(if let Some(inner) = bare.strip_prefix('<') {
                placeholder_value(inner.trim_end_matches('>'))
            } else if bare.starts_with('$') {
                "x".to_owned()
            } else {
                bare.to_owned()
            })
        })
        .collect()
}

/// The dummy value a `<placeholder>` stands for.
///
/// `<a|b>` lists the values the caller may pick from, so it stands for the
/// first of them: a value-enum flag must still parse. A name ending in `id`
/// or `ref` (`<decision-id>`, `<group_ref>`) stands for a number, which is
/// what those arguments take. Anything else is free text.
fn placeholder_value(name: &str) -> String {
    if name.contains('|') {
        return name.split('|').next().unwrap_or("x").to_owned();
    }
    if name.ends_with("id") || name.ends_with("ref") {
        return "1".to_owned();
    }
    "x".to_owned()
}

/// Every Markdown file in the repository whose prose ships to a reader:
/// the two root documents and everything under `docs/`.
///
/// Read from disk rather than embedded. The command catalog is
/// `include_str!`-ed into the binary because it is a shipping artifact;
/// documentation is not, and `Cargo.toml`'s `exclude` keeps `docs/` out of
/// the published crate entirely.
fn doc_files() -> Vec<PathBuf> {
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

/// Whether an invocation is prose about command shape rather than a command.
///
/// `ivar <verb>` and `ivar <group> <verb>` document the surface's grammar.
/// `argv` turns each placeholder into `x`, which `clap` rightly rejects as an
/// unknown subcommand — so these are dropped before they reach it. A
/// trailing `…` is dropped by `argv`, so `ivar feature execute …` reaches
/// clap as `ivar feature execute` — a group named without a verb, which
/// clap answers with help and the not-drift set forgives. A citation with
/// even one real token, like `ivar feature promote <repo>`, is a genuine
/// invocation and is kept.
fn is_placeholder_only(invocation: &str) -> bool {
    let mut tokens = invocation.split_whitespace();
    if tokens.next() != Some("ivar") {
        return false;
    }
    let rest: Vec<&str> = tokens.collect();
    if rest.is_empty() {
        return false;
    }
    if rest
        .iter()
        .all(|token| token.starts_with('<') || token.starts_with('$') || *token == "…")
    {
        return true;
    }
    false
}

/// Every citation in the shipped commands, the managed `HALL.md` block,
/// and repository documentation.
fn all_citations() -> Vec<Citation> {
    let mut found: Vec<Citation> = catalog()
        .iter()
        .flat_map(|command| citations(&format!("{}.md", command.id), command.content))
        .collect();
    found.extend(
        ivar::harness::skills::catalog()
            .iter()
            .filter(|skill| matches!(skill.id, "explore" | "automate"))
            .flat_map(|skill| {
                skill.files.iter().flat_map(move |file| {
                    citations(
                        &format!("{}/{}", skill.skill_dir_name(), file.path),
                        file.content,
                    )
                })
            }),
    );

    let hall = HallName::new("hall").unwrap();
    let repos = [RepoName::new("repo").unwrap()];
    let block = build_block(&hall, &repos);
    found.extend(citations("HALL.md managed block", &block));

    for path in doc_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        let label = path
            .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(&path)
            .display()
            .to_string();
        found.extend(citations(&label, &text));
    }
    found
}

/// A broken extractor would make every assertion below vacuous, so the
/// count is asserted too. 156 spans across the catalog, the `HALL.md`
/// block and 19 Markdown files; the floor is deliberately loose, to catch
/// "matched nothing" rather than to pin a number.
#[test]
fn prose_cites_invocations() {
    let found = all_citations();
    assert!(
        found.len() >= 120,
        "expected at least 120 quoted invocations, found {} — the extractor is broken",
        found.len()
    );
}

#[test]
fn the_explore_skill_citations_are_among_the_validated_sources() {
    let found = all_citations();
    assert!(
        found
            .iter()
            .any(|citation| citation.source == "ivar-explore/SKILL.md"
                && citation.text.starts_with("ivar session convert")),
        "the explore skill's invocations must be scanned"
    );
}

/// The repository's own prose is scanned, not just the shipped catalog.
/// `docs/reference/commands.md` is the file most likely to cite a verb, so
/// its absence from the scan is the regression this guards.
#[test]
fn repository_markdown_is_among_the_sources() {
    let scanned = doc_files();
    let names: Vec<String> = scanned
        .iter()
        .map(|path| path.display().to_string())
        .collect();

    for expected in [
        "README.md",
        "ARCHITECTURE.md",
        "docs/reference/commands.md",
        "docs/glossary.md",
        "docs/reference/limitations.md",
    ] {
        assert!(
            names.iter().any(|name| name.ends_with(expected)),
            "{expected} must be scanned, got {names:?}"
        );
    }
}

/// Prose about command *shape* is not an invocation. `argv` maps `<verb>`
/// to `x`, which parses as an unknown subcommand, so a citation made only
/// of placeholders has to be dropped before it reaches `clap`.
#[test]
fn a_citation_of_only_placeholders_is_not_an_invocation() {
    assert!(is_placeholder_only("ivar <verb>"));
    assert!(is_placeholder_only("ivar <group> <verb>"));
    assert!(
        !is_placeholder_only("ivar feature execute …"),
        "a trailing ellipsis is dropped by `argv`, not skipped here"
    );
    assert_eq!(
        argv("ivar feature execute …"),
        ["ivar", "feature", "execute"]
    );
    assert!(
        !is_placeholder_only("ivar feature promote <repo>"),
        "a real prefix with a placeholder argument is still an invocation"
    );
    assert!(!is_placeholder_only("ivar sync"));
}

#[test]
fn every_cited_invocation_parses() {
    let mut rejected = Vec::new();

    for citation in all_citations() {
        // `try_get_matches_from` consumes and mutates the `Command`, so
        // each citation gets a fresh tree.
        let command = Cli::command();
        let result = command.try_get_matches_from(argv(&citation.text));

        if let Err(error) = result {
            // Three kinds are not drift.
            //
            // `--help` and `--version` short-circuit parsing with a
            // display request: the invocation is valid, `clap` is just
            // telling us it would print instead of run.
            //
            // `MissingRequiredArgument` means the subcommand exists and
            // `clap` knows its arguments — it is the parser confirming the
            // path, not rejecting it. Prose names commands as commands
            // (``ivar feature deliver` refuses until...`), and demanding a
            // placeholder there would force every mention to carry fake
            // arguments. R-DRIFT is about citing what does not exist:
            // unknown subcommands and unknown flags.
            let not_drift = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp
                    | clap::error::ErrorKind::DisplayVersion
                    | clap::error::ErrorKind::MissingRequiredArgument
                    | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            if !not_drift {
                rejected.push(format!(
                    "  {} cites `{}`\n    clap: {}",
                    citation.source,
                    citation.text,
                    error.kind_message_first_line()
                ));
            }
        }
    }

    assert!(
        rejected.is_empty(),
        "shipped prose cites {} invocation(s) the CLI rejects:\n{}",
        rejected.len(),
        rejected.join("\n")
    );
}

/// `clap`'s `Display` is a multi-line, coloured help block; the first line
/// is the part that names the problem.
trait FirstLine {
    fn kind_message_first_line(&self) -> String;
}

impl FirstLine for clap::Error {
    fn kind_message_first_line(&self) -> String {
        self.to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    }
}
