#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::feature::RunId;
use crate::error::Status;
use crate::test_support::canonical_temp_dir as hall_root;

/// A fresh temp directory, canonicalised once so every path built from it
/// in a test already matches what `Layout::discover`'s own internal
/// canonicalisation will produce — tempdirs commonly sit behind a symlink
/// (`/tmp` -> `/private/tmp` on macOS), and comparing a raw path against a
/// canonicalised one would fail for reasons that have nothing to do with
/// the behaviour under test.

// -- discover -------------------------------------------------------------

#[test]
fn discover_finds_a_hall_at_the_starting_directory() {
    let (_guard, root) = hall_root();
    std::fs::write(root.join(MANIFEST_FILE_NAME), "{}").expect("write manifest");

    let layout = Layout::discover(&root)
        .expect("discover succeeds")
        .expect("hall is found");
    assert_eq!(layout.root(), root.as_path());
}

#[test]
fn discover_finds_a_hall_from_several_levels_down() {
    let (_guard, root) = hall_root();
    std::fs::write(root.join(MANIFEST_FILE_NAME), "{}").expect("write manifest");
    let nested = root.join("a").join("b").join("c");
    std::fs::create_dir_all(&nested).expect("create nested dirs");

    let layout = Layout::discover(&nested)
        .expect("discover succeeds")
        .expect("hall is found");
    assert_eq!(layout.root(), root.as_path());
}

#[test]
fn discover_returns_none_above_any_hall() {
    let (_guard, root) = hall_root();
    let nested = root.join("no-hall-anywhere-near-here");
    std::fs::create_dir_all(&nested).expect("create nested dir");

    let found = Layout::discover(&nested).expect("discover succeeds");
    assert!(found.is_none());
}

#[test]
fn discover_does_not_mistake_a_directory_named_ivar_json_for_the_manifest() {
    let (_guard, root) = hall_root();
    std::fs::write(root.join(MANIFEST_FILE_NAME), "{}").expect("write the real manifest");
    let nested = root.join("inner");
    std::fs::create_dir_all(nested.join(MANIFEST_FILE_NAME))
        .expect("create a directory named ivar.json");

    let layout = Layout::discover(&nested)
        .expect("discover succeeds")
        .expect("hall is found one level up, past the directory");
    assert_eq!(layout.root(), root.as_path());
}

#[test]
fn discover_fails_when_the_starting_path_does_not_exist() {
    let (_guard, root) = hall_root();
    let missing = root.join("does-not-exist");

    let error = Layout::discover(&missing).expect_err("missing path is unresolvable");
    assert!(matches!(error, DiscoverError::Unresolvable { .. }));
}

#[test]
fn discover_error_converts_to_a_blocked_failure_pointing_at_the_path() {
    let (_guard, root) = hall_root();
    let missing = root.join("does-not-exist");
    let error = Layout::discover(&missing).expect_err("missing path is unresolvable");

    let failure: Failure = error.into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "layout.discover_unresolvable");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);
}

// -- gitignore_lines --------------------------------------------------------

/// `.ivar/` (the whole-directory form) is wrong, and the failure is
/// invisible until someone goes looking for it: git prunes an *ignored
/// directory* before it ever evaluates a negation for anything inside it,
/// so `!.ivar/skills/` (or `!.ivar/setups/`) after a bare `.ivar/` would
/// never re-include the hall's committed skills or setup scripts. No
/// error, no warning — a teammate just clones the hall and those
/// directories are not there. `.ivar/*` excludes each direct child as its
/// own entry instead of the directory as a whole, which is exactly what
/// lets the negations below reach in and un-ignore specific children.
#[test]
fn gitignore_lines_excludes_the_dotdir_per_entry_and_reincludes_committed_children() {
    assert_eq!(
        Layout::gitignore_lines(),
        vec![
            ".ivar/*",
            "!.ivar/skills/",
            "!.ivar/setups/",
            ".claude/commands/ivar-*.md",
            ".opencode/commands/ivar-*.md",
            ".claude/skills/",
            ".opencode/skills/",
        ]
    );
}

/// The ignore contract for generated commands: only files matching the
/// reserved `ivar-*` prefix are ignored, never the command directory as a
/// whole — a user's own commands must stay committable.
#[test]
fn gitignore_lines_ignore_only_ivar_shipped_commands() {
    assert_eq!(
        Layout::gitignore_lines(),
        vec![
            ".ivar/*",
            "!.ivar/skills/",
            "!.ivar/setups/",
            ".claude/commands/ivar-*.md",
            ".opencode/commands/ivar-*.md",
            ".claude/skills/",
            ".opencode/skills/",
        ]
    );
}

/// Each provider's project workflow commands live in that provider's
/// native location, not under `.ivar/` — they are derived state for the
/// harness, exactly like the managed block and MCP config.
#[test]
fn command_dirs_use_each_providers_native_location() {
    let layout = Layout::at(Utf8PathBuf::from("/hall"));
    assert_eq!(
        layout.commands_dir(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/.claude/commands")
    );
    assert_eq!(
        layout.commands_dir(&Provider::OpenCode),
        Utf8PathBuf::from("/hall/.opencode/commands")
    );
}

// -- path accessors -----------------------------------------------------

#[test]
fn accessors_compute_the_documented_paths() {
    let layout = Layout::at("/hall");
    let repo = RepoName::new("api").unwrap();
    let branch = BranchName::new("main").unwrap();
    let feature = FeatureName::new("checkout").unwrap();
    let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();

    assert_eq!(layout.manifest(), Utf8PathBuf::from("/hall/ivar.json"));
    assert_eq!(layout.state(), Utf8PathBuf::from("/hall/.ivar/state.json"));
    assert_eq!(
        layout.repo_bare(&repo),
        Utf8PathBuf::from("/hall/.ivar/repos/api/.bare")
    );
    assert_eq!(
        layout.repo_worktree(&repo, &branch),
        Utf8PathBuf::from("/hall/.ivar/repos/api/main")
    );
    assert_eq!(
        layout.feature_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout")
    );
    assert_eq!(
        layout.execution_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/execution")
    );
    assert_eq!(
        layout.integration_candidate(&feature, &repo),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/integration/api/candidate")
    );
    assert_eq!(
        layout.integration_source(&feature, &repo),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/integration/api/source")
    );
    assert_eq!(
        layout.planning_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/planning")
    );
    assert_eq!(
        layout.feature_session(&feature, &session),
        Utf8PathBuf::from(
            "/hall/.ivar/features/checkout/sessions/2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c"
        )
    );
    assert_eq!(
        layout.feature_sessions_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/sessions")
    );
    assert_eq!(
        layout.discovery_session(&session),
        Utf8PathBuf::from("/hall/.ivar/sessions/2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c")
    );
    assert_eq!(
        layout.setup_script(&repo),
        Utf8PathBuf::from("/hall/.ivar/setups/api.sh")
    );
    assert_eq!(
        layout.hall_skills(),
        Utf8PathBuf::from("/hall/.ivar/skills")
    );
    assert_eq!(layout.repos_dir(), Utf8PathBuf::from("/hall/.ivar/repos"));
    assert_eq!(
        layout.repo_dir(&repo),
        Utf8PathBuf::from("/hall/.ivar/repos/api")
    );
    assert_eq!(
        layout.features_dir(),
        Utf8PathBuf::from("/hall/.ivar/features")
    );
    assert_eq!(
        layout.discovery_sessions_dir(),
        Utf8PathBuf::from("/hall/.ivar/sessions")
    );
    assert_eq!(
        layout.instruction_alias(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/CLAUDE.md")
    );
    assert_eq!(
        layout.instruction_alias(&Provider::OpenCode),
        Utf8PathBuf::from("/hall/AGENTS.md")
    );
    assert_eq!(
        layout.hall_instructions(),
        Utf8PathBuf::from("/hall/HALL.md")
    );
    assert_eq!(
        layout.mcp_config(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/.mcp.json")
    );
    assert_eq!(
        layout.mcp_config(&Provider::OpenCode),
        Utf8PathBuf::from("/hall/opencode.json")
    );
    assert_eq!(
        layout.plan_dir(&feature),
        Utf8PathBuf::from("/hall/plans/checkout")
    );
    assert_eq!(
        layout.work_dir(&feature),
        Utf8PathBuf::from("/hall/docs/checkout")
    );
    assert_eq!(
        layout.discovery_doc(&feature),
        Utf8PathBuf::from("/hall/docs/checkout/discovery.md")
    );
    assert_eq!(
        layout.research_dir(&feature),
        Utf8PathBuf::from("/hall/docs/checkout/research")
    );
}

/// The regression guard for the bug `domain::provider` exists to make
/// impossible: `Provider::ClaudeCode` maps to `.claude`, not `.claude-code`.
/// Written as an explicit literal, not parameterised over the id, so this
/// test cannot pass by accident if `harness_dir` goes back to interpolating
/// `provider.id()`.
#[test]
fn harness_dir_maps_claude_code_to_dot_claude_not_dot_claude_code() {
    let layout = Layout::at("/hall");

    assert_eq!(
        layout.harness_dir(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/.claude")
    );
    assert_eq!(
        layout.commands_dir(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/.claude/commands")
    );
}

#[test]
fn repo_worktree_nests_a_slash_containing_branch_name() {
    let layout = Layout::at("/hall");
    let repo = RepoName::new("api").unwrap();
    let branch = BranchName::new("feat/auth-v2").unwrap();

    assert_eq!(
        layout.repo_worktree(&repo, &branch),
        Utf8PathBuf::from("/hall/.ivar/repos/api/feat/auth-v2")
    );
}

/// The canonical hall instructions and each provider's root alias are
/// distinct paths — one file is the source, the others are symlinks to it.
/// A single ambiguous accessor is exactly what let the two be confused.
#[test]
fn hall_instructions_and_provider_aliases_are_distinct() {
    let layout = Layout::at(Utf8PathBuf::from("/hall"));

    assert_eq!(
        layout.hall_instructions(),
        Utf8PathBuf::from("/hall/HALL.md")
    );
    assert_eq!(
        layout.instruction_alias(&Provider::ClaudeCode),
        Utf8PathBuf::from("/hall/CLAUDE.md")
    );
    assert_eq!(
        layout.instruction_alias(&Provider::OpenCode),
        Utf8PathBuf::from("/hall/AGENTS.md")
    );
}

// -- run receipt paths ----------------------------------------------------

/// Every path a Run Receipt lives at, so no filename arithmetic has to happen
/// in `store::feature::run` — or, worse, in an action.
#[test]
fn run_receipt_accessors_compute_the_documented_paths() {
    let layout = Layout::at("/hall");
    let feature = FeatureName::new("checkout").unwrap();
    let run = RunId::new("6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a33").unwrap();

    assert_eq!(
        layout.run_receipt(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/run.json")
    );
    assert_eq!(
        layout.run_archive_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/archive")
    );
    assert_eq!(
        layout.run_archive_runs_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/archive/runs")
    );
    assert_eq!(
        layout.archived_run(&feature, &run),
        Utf8PathBuf::from(
            "/hall/.ivar/features/checkout/execution/archive/runs/\
             6f1d9e64-0d1a-4f2b-9a5c-2b7e1d4c8a33.json"
        )
    );
    assert_eq!(
        layout.board_archive_dir(&feature),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/archive/boards")
    );
    assert_eq!(
        layout.archived_board(&feature, "abc123"),
        Utf8PathBuf::from("/hall/.ivar/features/checkout/execution/archive/boards/abc123.json")
    );
}

/// The archive sits under the execution directory, not beside it: `feature
/// close` and the read-only guard both reason about one directory per feature
/// concern, and a receipt archive somewhere else would be missed by both.
#[test]
fn the_run_archive_lives_under_the_execution_directory() {
    let layout = Layout::at("/hall");
    let feature = FeatureName::new("checkout").unwrap();
    let execution = layout.execution_dir(&feature);

    for path in [
        layout.run_receipt(&feature),
        layout.run_archive_dir(&feature),
        layout.run_archive_runs_dir(&feature),
        layout.board_archive_dir(&feature),
        layout.archived_board(&feature, "abc123"),
    ] {
        assert!(
            path.starts_with(&execution),
            "{path} must live under {execution}"
        );
    }
}

/// `archived_run` takes a [`RunId`], not a `&str`, and `RunId`'s only
/// constructor validates it as a UUID. That is what stops `status --run <id>`
/// from becoming a path traversal — the check is in the type, so no call site
/// can forget it.
#[test]
fn a_run_id_cannot_carry_a_path_traversal_into_the_archive() {
    for hostile in ["../../../etc/passwd", "..", "a/b", ""] {
        assert!(
            RunId::new(hostile).is_err(),
            "`{hostile}` must never become a run id"
        );
    }
}

/// ADR-0002: memory and execution are two homes joined by one name. A
/// regression that collapsed them — `work_dir` returning the plan path, or
/// `plan_dir` moving under `docs/` — would pass every other test here.
#[test]
fn memory_and_execution_are_separate_homes_for_one_name() {
    let layout = Layout::at("/hall");
    let feature = FeatureName::new("checkout").unwrap();

    assert_ne!(layout.work_dir(&feature), layout.plan_dir(&feature));
    assert!(
        layout
            .plan_dir(&feature)
            .starts_with(Utf8PathBuf::from("/hall/plans"))
    );
    assert_eq!(layout.work_docs_root(), Utf8PathBuf::from("/hall/docs"));
    assert!(
        layout
            .work_dir(&feature)
            .starts_with(layout.work_docs_root())
    );
    assert!(
        layout
            .discovery_doc(&feature)
            .starts_with(layout.work_dir(&feature))
    );
    assert!(
        layout
            .research_dir(&feature)
            .starts_with(layout.work_dir(&feature))
    );
}
