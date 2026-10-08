//! Text checks over the workflow commands and skills shipped from
//! `src/harness/`, and over every `ivar …` invocation that shipped prose or
//! repository documentation cites.
//!
//! Nothing here spawns the binary: each check reads files or parses with
//! `clap`, so it also runs in the docs-only CI job.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

/// Every shipped command id, as `/ivar-<id>`.
const SHIPPED_IDS: [&str; 12] = [
    "connect",
    "discovery",
    "feature-cleanup",
    "feature-create",
    "feature-status",
    "promote",
    "relations",
    "repo-list",
    "repo-setup",
    "review",
    "sync",
    "workspace",
];

/// Shipped skill `ivar-execute` aligns with the execution run receipt lifecycle
/// and completion branching: starting the run in preparation, finishing before
/// completion, and branching to integrate for subfeatures or deliver for root features.
#[test]
fn ivar_execute_skill_contains_execute_start_and_finish_commands() {
    let skill_path = format!(
        "{}/src/harness/skills/ivar-execute/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(skill_path).unwrap();

    assert!(
        content.contains("ivar feature execute start"),
        "skill must document starting the run receipt during preparation"
    );
    assert!(
        content.contains("ivar feature execute finish"),
        "skill must document finishing the run receipt before integration/delivery"
    );
    assert!(
        content.contains("ivar feature integrate"),
        "skill must document integrating subfeatures upon completion"
    );
    assert!(
        content.contains("ivar feature deliver"),
        "skill must document delivering root features upon completion"
    );
    assert!(
        content.contains("is_subfeature"),
        "skill must document branching on subfeature status"
    );
}

#[test]
fn ivar_execute_skill_finishes_the_run_with_a_report_and_outcome() {
    let skill_path = format!(
        "{}/src/harness/skills/ivar-execute/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(skill_path).unwrap();

    assert!(
        content.contains("ivar feature execute finish <feature> --report-json <path> --outcome"),
        "skill must show the flags finish requires"
    );
    assert!(
        content.contains("ivar feature execute finish --print-schema"),
        "skill must point at the report schema"
    );
}

#[test]
fn ivar_execute_skill_delivers_root_features_through_the_deliver_skill() {
    let skill_path = format!(
        "{}/src/harness/skills/ivar-execute/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(skill_path).unwrap();

    assert!(
        content.contains("Load the `ivar-deliver` skill"),
        "root feature delivery must hand off to the ivar-deliver skill"
    );
}

/// Wave progress goes into the run receipt; editing `plan.md` mid-run moves
/// its fingerprint and diverges the run.
#[test]
fn ivar_execute_skill_records_progress_through_execute_checkpoint_only() {
    let base = env!("CARGO_MANIFEST_DIR");
    let execute =
        std::fs::read_to_string(format!("{base}/src/harness/skills/ivar-execute/SKILL.md"))
            .unwrap();
    let plan =
        std::fs::read_to_string(format!("{base}/src/harness/skills/ivar-plan/SKILL.md")).unwrap();

    assert!(execute.contains("ivar feature execute checkpoint"));
    assert!(execute.contains("Never edit `plan.md` during a run"));
    for stale in [
        "records progress and deferred validation failures in `plan.md`",
        "record in plan.md",
        "record completed wave in plan.md",
        "for this wave in `plan.md`",
        "update `plan.md`",
    ] {
        assert!(!execute.contains(stale), "ivar-execute still says: {stale}");
    }
    assert!(!plan.contains("marks each wave complete in"));
}

#[test]
fn no_shipped_command_tells_the_agent_to_export_ivar_vars() {
    for id in SHIPPED_IDS {
        let source = format!(
            "{}/src/harness/commands/{id}.md",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(source).unwrap();
        assert!(
            !body.contains("export IVAR_"),
            "/ivar-{id} still tells the agent to export IVAR_* vars"
        );
    }
}

#[test]
fn shipped_commands_do_not_reference_old_slash_execute() {
    for id in SHIPPED_IDS {
        let source = format!(
            "{}/src/harness/commands/{id}.md",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(source).unwrap();
        assert!(
            !body.contains("/ivar-execute"),
            "/ivar-{id} still references /ivar-execute"
        );
    }
}

/// The deliver skill documents PR metadata: global/scoped syntax,
/// inline vs file body, title guidance, and land conflict.
#[test]
fn deliver_skill_documents_pr_metadata() {
    let source = format!(
        "{}/src/harness/skills/ivar-deliver/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let body = std::fs::read_to_string(source).unwrap();

    assert!(
        body.contains("--name"),
        "deliver should document --name flag"
    );
    assert!(
        body.contains("--body"),
        "deliver should document --body flag"
    );
    assert!(
        body.contains("--repo api"),
        "deliver should show scoped --repo syntax"
    );
    assert!(
        body.contains("--repo web"),
        "deliver should show multiple scoped repos"
    );
    assert!(
        body.contains("--draft"),
        "deliver should document --draft flag"
    );
    assert!(
        body.contains("./notes.md"),
        "deliver should document file-relative body syntax"
    );
    assert!(
        body.contains("<type>: <short message>"),
        "deliver should guide <type>: <short message> title format"
    );
    assert!(
        body.contains("cannot be used with `--land`"),
        "deliver should state metadata conflicts with land mode"
    );
    assert!(
        !body.contains("L-1234"),
        "deliver should not use a Linear identifier as an example title"
    );
}

#[test]
fn deliver_skill_documents_only_selection() {
    let source = format!(
        "{}/src/harness/skills/ivar-deliver/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let body = std::fs::read_to_string(source).unwrap();

    assert!(body.contains("--only"), "deliver should document --only");
    assert!(
        body.contains("merged"),
        "deliver should explain the already-merged PR case"
    );
}

/// Every `ivar ...` invocation quoted in shipped prose and repository
/// documentation must parse against the real CLI.
///
/// `tests/docs_reference.rs` keeps the *generated* half of the docs honest by
/// rendering `clap` and failing on disagreement. This is the same rule for the
/// hand-written half: prose cites invocations, and `clap` is the only
/// authority on whether one is real.
///
/// Existence is not the interesting part — every subcommand cited today
/// exists. Arity is: `ivar session convert <a> <b>` names a real subcommand
/// and is still wrong, because `convert` takes one positional. Feeding the
/// whole line to `clap` is what catches that. Citing a group without a verb
/// (`` `ivar repo` ``) is how a heading names a group; clap answers that with
/// help rather than a rejection, which the not-drift set forgives.
mod cited_invocations {
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
                Some(if bare.starts_with('<') || bare.starts_with('$') {
                    "x".to_owned()
                } else {
                    bare.to_owned()
                })
            })
            .collect()
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
}

#[test]
fn execute_and_deliver_skills_forbid_ai_attribution() {
    for skill in ["ivar-execute", "ivar-deliver"] {
        let source = format!(
            "{}/src/harness/skills/{skill}/SKILL.md",
            env!("CARGO_MANIFEST_DIR")
        );
        let body = std::fs::read_to_string(source).unwrap();
        assert!(
            body.contains("Never add AI attribution"),
            "{skill} should forbid AI attribution"
        );
    }
    let deliver = std::fs::read_to_string(format!(
        "{}/src/harness/skills/ivar-deliver/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(
        deliver.contains("deliver.ai_attribution"),
        "deliver should name the refusal code"
    );
}

fn shipped(path: &str) -> String {
    std::fs::read_to_string(format!("{}/src/harness/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn every_review_loop_in_execute_has_a_round_cap() {
    let execute = shipped("skills/ivar-execute/SKILL.md");
    assert!(
        !execute.contains("until 0 findings"),
        "goal mode must not demand zero smells"
    );
    assert!(
        execute.matches("at most").count() >= 2,
        "default and goal review loops both need a cap"
    );
    assert!(!execute.contains('┌'), "the diagram duplicates Phases 1-4");
    assert!(
        execute.contains("## Recovery"),
        "execute must document recovery"
    );
    for case in ["usage limit", "subagent", "CI fails after deliver"] {
        assert!(execute.contains(case), "recovery must cover: {case}");
    }
    assert!(shipped("skills/ivar-execute/references/subagent.md").contains("Design ref"));
}

#[test]
fn subfeatures_orchestrates_children_with_subagents_from_the_parent_session() {
    let skill = shipped("skills/ivar-subfeatures/SKILL.md");
    // The manual one-terminal-per-child flow is gone.
    assert!(!skill.contains("ivar session sandbox --session"));
    assert!(!skill.contains("never launch a provider yourself"));
    // Portable subagent contract: one level, return-only.
    for rule in [
        "One level of subagents",
        "never message a running subagent",
        "Absolute paths only",
        "needs_input",
        "Wave 0",
        "references/child-planner.md",
        "references/state.md",
        "ivar-execute/references/subagent.md",
    ] {
        assert!(skill.contains(rule), "subfeatures must state: {rule}");
    }
    // Every provider names its dispatch mechanism.
    for provider in ["Claude Code", "OpenCode", "omp"] {
        assert!(skill.contains(provider), "dispatch for {provider}");
    }
    // The child lifecycle, in order.
    let order = [
        "ivar feature create <child> --parent <parent>",
        "ivar plan approve <child> plan",
        "ivar session start <child> --detached --json",
        "ivar feature execute start <child> --mode goal",
        "ivar feature execute finish <child>",
        "ivar session stop <session-id>",
        "ivar feature integrate <child>",
        "ivar feature deliver <parent> --preview",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|step| {
            skill
                .find(step)
                .unwrap_or_else(|| panic!("missing: {step}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "lifecycle out of order: {positions:?}"
    );
    // It stops before applying delivery.
    assert!(!skill.contains("ivar feature deliver <parent> --fingerprint"));

    let planner = shipped("skills/ivar-subfeatures/references/child-planner.md");
    for field in [
        "status: done",
        "status: needs_input",
        "questions:",
        "### Decisions",
        "absolute",
    ] {
        assert!(
            planner.contains(field),
            "child-planner must define: {field}"
        );
    }
    let state = shipped("skills/ivar-subfeatures/references/state.md");
    for column in [
        "plan_gate",
        "run",
        "sessions",
        "last_wave",
        "needs_revision",
        "diverged",
    ] {
        assert!(state.contains(column), "state table must read: {column}");
    }
}

#[test]
fn ivar_execute_skill_supports_explicit_feature_and_orchestrator_handoff() {
    let execute = shipped("skills/ivar-execute/SKILL.md");

    // Frontmatter argument-hint must take optional feature
    assert!(
        execute.contains("argument-hint: [feature] [plan-path] [--mode goal|default]"),
        "ivar-execute must declare [feature] in argument-hint"
    );

    // Phase 1.1 feature resolution & plan default
    assert!(
        execute.contains("Resolve the target feature from `$ARGUMENTS`"),
        "Phase 1.1 must resolve target feature from arguments with fallback"
    );
    assert!(
        execute.contains("`.ivar/features/<feature>/plan.md`"),
        "Phase 1.1 default plan must be relative to hall root"
    );

    // Phase 4.2 report path
    assert!(
        execute.contains(".tmp/<feature>-run-report.json"),
        "Phase 4.2 report filename must include feature name"
    );

    // Phase 4.4 hand-off branching for child-session vs parent orchestrator
    assert!(
        execute.contains("If this session belongs to the parent"),
        "Phase 4.4 must handle orchestrator returning control to ivar-subfeatures"
    );
    assert!(
        execute.contains("Stop this session, then integrate from the parent session"),
        "Phase 4.4 must retain stop instruction when session belongs to child"
    );

    // Recovery detached session startup
    assert!(
        execute.contains("Root feature driven by `ivar-subfeatures`"),
        "Phase 4.4 must return a parent Wave 0 run to the orchestrator"
    );
    assert!(
        execute.contains("ivar session start <feature> --detached"),
        "Recovery must instruct orchestrator to start detached child session"
    );
}

#[test]
fn phase_transitions_offer_approve_and_continue_and_templates_match_execute() {
    for path in ["skills/ivar-plan/SKILL.md", "skills/ivar-execute/SKILL.md"] {
        assert!(
            shipped(path).contains("Approve and continue"),
            "{path} needs a combined gate"
        );
    }
    let template = shipped("skills/ivar-plan/references/plan-template.md");
    for stale in [
        "Deferred validation failures",
        "Human approval",
        "✅",
        "| Done |",
    ] {
        assert!(
            !template.contains(stale),
            "plan-template still tracks progress: {stale}"
        );
    }
    assert!(shipped("skills/ivar-plan/references/task-template.md").contains("**Design ref:**"));
    assert!(shipped("skills/ivar-plan/SKILL.md").contains("On omp"));
    assert!(shipped("skills/ivar-deliver/SKILL.md").contains("## After delivery"));
}

#[test]
fn shipped_text_names_only_env_vars_ivar_exports() {
    for path in [
        "commands/feature-cleanup.md",
        "commands/discovery.md",
        "commands/connect.md",
    ] {
        assert!(
            !shipped(path).contains("IVAR_SESSION_TYPE"),
            "{path} uses an unset variable"
        );
    }
    assert!(
        !shipped("commands/discovery.md").contains("look in `.claude/skills/`"),
        "a view dir never holds the hall's ivar-plan skill"
    );
    assert!(
        shipped("commands/connect.md").contains("Claude Code"),
        "workdir is opencode/omp only"
    );
    assert!(shipped("commands/sync.md").contains(".omp/commands/"));
}
