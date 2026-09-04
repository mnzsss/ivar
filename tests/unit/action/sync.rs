#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::mcp::McpServerDef;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::harness::{commands, config};
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};
use camino::Utf8Path;

/// A hall with `repos` already declared in its `ivar.json`, plus the
/// origins those repos point at. Returns the hall root and the scratch dir
/// guard.
fn hall_with(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    if !repos.is_empty() {
        let origins = root.parent().unwrap().join("origins");
        let declared: Vec<Repo> = repos
            .iter()
            .map(|(name, branch)| {
                let origin = seeded_repo(&origins.join(name), branch);
                Repo::new(
                    RepoName::new(*name).unwrap(),
                    origin.as_str(),
                    BranchName::new(*branch).unwrap(),
                )
            })
            .collect();

        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            declared,
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
    }

    (guard, root)
}

fn entry<'a>(outcome: &'a SyncOutcome, surface: &str, label: &str) -> &'a Entry {
    outcome
        .entries
        .iter()
        .find(|e| e.surface == surface && e.label == label)
        .unwrap_or_else(|| panic!("no `{surface}` / `{label}` entry in {:?}", outcome.entries))
}

// -- the empty hall --------------------------------------------------------

#[test]
fn syncing_a_hall_with_no_repos_sets_up_the_skeleton_and_the_managed_block() {
    let (_guard, root) = hall_with(&[]);
    // Simulate a hall whose instructions were never materialised (or were
    // deleted by hand): `sync` is what creates the canonical file and the
    // first alias.
    fs::remove_file(&root.join("HALL.md")).unwrap();
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(fs::is_dir(&root.join(".ivar/repos")).unwrap());
    assert_eq!(
        entry(&report.value, "hall", "HALL.md").change,
        Change::Created
    );
    assert_eq!(
        entry(&report.value, "claude-code", "CLAUDE.md alias").change,
        Change::Created
    );
    let block = fs::read_text(&root.join("HALL.md")).unwrap().unwrap();
    assert!(block.contains("# acme"));
    assert_eq!(
        fs::read_symlink(&root.join("CLAUDE.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md")),
        "the enabled provider's alias must be a relative symlink to HALL.md"
    );
}

/// `sync` runs after every `git pull`. The second run must touch nothing,
/// or every run leaves a spurious modification in `git status`.
#[test]
fn a_second_sync_changes_nothing() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();
    let before = fs::read_bytes(&root.join("HALL.md")).unwrap().unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(
        report
            .value
            .entries
            .iter()
            .all(|e| e.change == Change::Unchanged),
        "expected every entry unchanged, got {:?}",
        report.value.entries
    );
    assert_eq!(
        fs::read_bytes(&root.join("HALL.md")).unwrap().unwrap(),
        before
    );
}

// -- repos -----------------------------------------------------------------

#[test]
fn a_declared_repo_is_cloned_bare_and_gets_its_default_branch_worktree() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "repo api", "bare clone").change,
        Change::Created
    );
    assert_eq!(
        entry(&report.value, "repo api", "worktree main").change,
        Change::Created
    );
    assert!(root.join(".ivar/repos/api/.bare/HEAD").is_file());
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/api/main/README.md")).unwrap(),
        "seed\n"
    );
}

#[test]
fn the_managed_block_lists_every_declared_repo() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root.clone());

    sync(&ctx, SyncInput::default()).unwrap();

    let block = fs::read_text(&root.join("HALL.md")).unwrap().unwrap();
    assert!(block.contains("`api`"));
    assert!(block.contains("`web`"));
}

/// The whole point of the warning channel: eight repos, one bad remote,
/// seven still set up and the process exits 1 rather than 2.
#[test]
fn an_unreachable_repo_becomes_a_warning_and_the_others_still_sync() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let layout = Layout::at(root.clone());
    let mut repos = Manifest::read(&layout).unwrap().unwrap().repos().to_vec();
    repos.push(Repo::new(
        RepoName::new("ghost").unwrap(),
        root.join("no-such-origin").as_str(),
        BranchName::new("main").unwrap(),
    ));
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(!report.is_clean(), "a failed repo must not be a clean run");
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].subject, "repo ghost");
    assert_eq!(
        entry(&report.value, "repo ghost", "bare clone").change,
        Change::Failed
    );
    // The healthy repo still landed.
    assert_eq!(
        entry(&report.value, "repo api", "bare clone").change,
        Change::Created
    );
    assert!(root.join(".ivar/repos/api/main/README.md").is_file());
}

/// git's own message for a non-empty clone target names the symptom, not
/// the cause. This says what is actually wrong and what to do about it.
#[test]
fn a_partial_clone_left_at_the_bare_path_is_named_rather_than_left_to_git() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let bare = root.join(".ivar/repos/api/.bare");
    fs::ensure_dir(&bare).unwrap();
    fs::write_text(&bare.join("leftover"), "junk").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    let failed = entry(&report.value, "repo api", "bare clone");
    assert_eq!(failed.change, Change::Failed);
    assert!(
        failed
            .detail
            .as_deref()
            .unwrap()
            .contains("is not a bare clone"),
        "was: {:?}",
        failed.detail
    );
}

/// The worktree twin of the case above. Both go through `occupied`, so this
/// is what keeps the shared helper honest about saying the *right* noun.
#[test]
fn something_else_at_the_worktree_path_is_named_rather_than_left_to_git() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let worktree = root.join(".ivar/repos/api/main");
    fs::ensure_dir(&worktree).unwrap();
    fs::write_text(&worktree.join("notes.md"), "mine").unwrap();
    let ctx = Ctx::new(root);

    let report = sync(&ctx, SyncInput::default()).unwrap();

    let failed = entry(&report.value, "repo api", "worktree");
    assert_eq!(failed.change, Change::Failed);
    assert!(
        failed
            .detail
            .as_deref()
            .unwrap()
            .contains("is not a worktree"),
        "was: {:?}",
        failed.detail
    );
}

/// `main` versus `master` is the commonest first-run mistake, and git's own
/// refusal names neither the manifest nor the branch that does exist.
#[test]
fn a_branch_the_repo_does_not_have_names_the_repos_default_instead() {
    let (_guard, root) = hall_with(&[("api", "master")]);
    // The origin is on `master`; declare `main` in the manifest instead.
    let layout = Layout::at(root.clone());
    let url = Manifest::read(&layout).unwrap().unwrap().repos()[0]
        .url()
        .to_owned();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            url,
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root);

    let report = sync(&ctx, SyncInput::default()).unwrap();

    let failed = entry(&report.value, "repo api", "worktree");
    assert_eq!(failed.change, Change::Failed);
    let detail = failed.detail.as_deref().unwrap();
    assert!(detail.contains("main"), "was: {detail}");
    assert!(
        detail.contains("master"),
        "the branch that DOES exist has to survive into the rendered sentence, \
             not sit in a field a per-item failure never renders: {detail}"
    );
    assert!(
        report.warnings.iter().any(|w| w.subject == "repo api"),
        "a named branch mismatch must still be a warning, not a silent skip"
    );
}

// -- setup scripts ---------------------------------------------------------

/// A setup script writes something a `git clone` never would — the whole
/// reason the hook exists.
fn write_setup_script(root: &Utf8Path, repo: &str, body: &str) {
    let script = Layout::at(root).setup_script(&RepoName::new(repo).unwrap());
    fs::ensure_dir(script.parent().unwrap()).unwrap();
    fs::write_text(&script, body).unwrap();
}

#[test]
fn a_repos_setup_script_runs_in_its_worktree_with_the_ivar_environment() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nset -euo pipefail\n\
             printf '%s %s %s\\n' \"$IVAR_REPO\" \"$IVAR_BRANCH\" \"$IVAR_WORKTREE_KIND\" > .ivar-setup-ran\n",
    );
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "repo api", "setup script").change,
        Change::Created
    );
    let evidence = root.join(".ivar/repos/api/main/.ivar-setup-ran");
    assert_eq!(
        std::fs::read_to_string(&evidence).unwrap(),
        "api main default\n"
    );
}

/// `sync` runs against the default worktree, where there is no feature —
/// so `IVAR_SECRETS_DIR` is set and `IVAR_FEATURE` deliberately is not.
#[test]
fn a_setup_script_gets_the_secrets_dir_and_no_feature_on_the_default_worktree() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nset -euo pipefail\n\
             printf '%s\\n%s\\n' \"$IVAR_SECRETS_DIR\" \"${IVAR_FEATURE:-<unset>}\" > .ivar-env\n",
    );
    let ctx = Ctx::new(root.clone());

    sync(&ctx, SyncInput::default()).unwrap();

    let evidence = std::fs::read_to_string(root.join(".ivar/repos/api/main/.ivar-env")).unwrap();
    let mut lines = evidence.lines();
    assert!(
        lines.next().unwrap().ends_with("/.ivar/secrets"),
        "was: {evidence}"
    );
    assert_eq!(lines.next().unwrap(), "<unset>");
}

/// The directory has to exist for a user to find it and drop a file in.
/// It is local and gitignored, so creating it promises a teammate nothing.
#[test]
fn sync_creates_the_secrets_dir() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());

    sync(&ctx, SyncInput::default()).unwrap();

    assert!(fs::is_dir(&root.join(".ivar/secrets")).unwrap());
}

#[test]
fn a_setup_script_does_not_run_twice_for_the_same_content() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
    );
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "repo api", "setup script").change,
        Change::Unchanged
    );
    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "x");
}

#[test]
fn changing_the_script_makes_it_run_again() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
    );
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf y >> .ivar-setup-runs\n",
    );
    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "repo api", "setup script").change,
        Change::Updated
    );
    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "xy");
}

#[test]
fn force_setup_runs_an_unchanged_script_again() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\n",
    );
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    sync(&ctx, SyncInput { force_setup: true }).unwrap();

    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "xx");
}

/// A failed setup that recorded "done" would leave every later sync
/// silently skipping the repair the user is waiting for.
#[test]
fn a_failing_setup_script_warns_and_is_retried_on_the_next_sync() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    write_setup_script(
        &root,
        "api",
        "#!/usr/bin/env bash\nprintf x >> .ivar-setup-runs\nexit 1\n",
    );
    let ctx = Ctx::new(root.clone());

    let first = sync(&ctx, SyncInput::default()).unwrap();
    assert!(!first.is_clean());
    assert_eq!(
        entry(&first.value, "repo api", "setup script").change,
        Change::Failed
    );

    let second = sync(&ctx, SyncInput::default()).unwrap();
    assert_eq!(
        entry(&second.value, "repo api", "setup script").change,
        Change::Failed
    );
    let runs = root.join(".ivar/repos/api/main/.ivar-setup-runs");
    assert_eq!(
        std::fs::read_to_string(&runs).unwrap(),
        "xx",
        "a failed setup must be retried, not remembered as done"
    );
}

#[test]
fn a_repo_with_no_setup_script_produces_no_setup_entry() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root);

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(
        !report
            .value
            .entries
            .iter()
            .any(|e| e.label == "setup script"),
        "expected no setup entry, got {:?}",
        report.value.entries
    );
}

// -- providers -------------------------------------------------------------

#[test]
fn a_disabled_providers_alias_entry_is_removed_entirely() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    // A stale regular AGENTS.md from when the hall did list OpenCode — with
    // the user's own text inside it. The disabled-provider rule removes the
    // whole entry, including a regular file; it never removes HALL.md.
    fs::write_text(
        &root.join("AGENTS.md"),
        &format!(
            "{}\nstale\n{}\n\n# House rules\n",
            config::MANAGED_START,
            config::MANAGED_END
        ),
    )
    .unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "AGENTS.md alias").change,
        Change::Removed
    );
    assert!(
        !fs::exists(&root.join("AGENTS.md")).unwrap(),
        "a disabled provider's alias path is entirely ivar-managed"
    );
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
}

#[test]
fn a_provider_the_hall_does_not_list_and_never_did_is_unchanged() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "AGENTS.md alias").change,
        Change::Unchanged
    );
    assert!(!fs::exists(&root.join("AGENTS.md")).unwrap());
}

// -- MCP config -----------------------------------------------------------

#[test]
fn sync_materialises_the_mcp_config_at_the_hall_root() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "claude-code", ".mcp.json MCP config").change,
        Change::Created
    );
    let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
    // Valid JSON, matching the empty-server v1 shape.
    let parsed: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
    assert_eq!(parsed, serde_json::json!({ "mcpServers": {} }));
}

/// `sync` runs after every `git pull`; the second run must leave the MCP
/// config byte-identical too.
#[test]
fn the_mcp_config_is_unchanged_on_a_second_sync() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();
    let before = fs::read_bytes(&root.join(".mcp.json")).unwrap().unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "claude-code", ".mcp.json MCP config").change,
        Change::Unchanged
    );
    assert_eq!(
        fs::read_bytes(&root.join(".mcp.json")).unwrap().unwrap(),
        before
    );
}

#[test]
fn sync_materialises_the_opencode_config_when_opencode_is_available() {
    let (_guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "opencode.json MCP config").change,
        Change::Created
    );
    let on_disk = fs::read_text(&root.join("opencode.json")).unwrap().unwrap();
    assert!(on_disk.contains("$schema"), "was: {on_disk}");
    assert!(on_disk.contains("\"mcp\": {}"), "was: {on_disk}");
}

#[test]
fn sync_strips_a_stale_mcp_config_for_a_provider_the_hall_dropped() {
    let (_guard, root) = hall_with(&[]);
    // A stale opencode.json from when the hall did list OpenCode — carrying
    // a user key that must survive the strip.
    fs::write_text(
        &root.join("opencode.json"),
        &serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": { "stale": { "type": "local", "command": ["old"] } },
        })
        .to_string(),
    )
    .unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "opencode.json MCP config").change,
        Change::Removed
    );
    let on_disk = fs::read_text(&root.join("opencode.json")).unwrap().unwrap();
    assert!(on_disk.contains("claude-sonnet-4-5"), "was: {on_disk}");
    assert!(!on_disk.contains("stale"), "was: {on_disk}");
}

#[test]
fn sync_writes_declared_servers_into_the_config() {
    let (_guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_mcp_servers(vec![McpServerDef::new("docs", "stdio").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
    assert!(on_disk.contains("\"acme-docs\""), "was: {on_disk}");
    assert!(on_disk.contains("\"npx\""), "was: {on_disk}");
}

/// Declaring a server while OMP is available must not reach OMP's renderer.
/// Task 03 gives OMP a native MCP document; until then `sync_mcp` skips it,
/// and this test is what keeps that skip honest.
#[test]
fn sync_with_omp_available_writes_other_providers_and_skips_omp() {
    let (_guard, root) = hall_with_all_providers();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let manifest = manifest
        .with_mcp_servers(vec![McpServerDef::new("docs", "stdio").command("npx")])
        .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    let on_disk = fs::read_text(&root.join(".mcp.json")).unwrap().unwrap();
    assert!(on_disk.contains("\"acme-docs\""), "was: {on_disk}");
    assert!(
        !fs::exists(&root.join(".omp/mcp.json")).unwrap(),
        "OMP has no MCP document until Task 03"
    );
}

// -- official workflow commands -------------------------------------------

/// A hall whose `ivar.json` lists all providers.
fn hall_with_all_providers() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with(&[]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode, Provider::Omp],
            Provider::ClaudeCode,
        ),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    (guard, root)
}

/// The embedded source of the shipped command `id`.
fn embedded(id: &str) -> String {
    commands::catalog()
        .iter()
        .find(|command| command.id == id)
        .unwrap()
        .content
        .to_owned()
}

#[test]
fn sync_materialises_shipped_commands_for_available_providers() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    for provider in Provider::ALL {
        let dir = root.join(provider.commands_dir());
        for command in commands::catalog() {
            assert!(
                fs::is_file(&dir.join(command.file_name())).unwrap(),
                "{} missing for {provider}",
                command.file_name()
            );
        }
    }
    // OpenCode's commands come from this sync — init only bootstrapped the
    // default provider, Claude Code.
    assert_eq!(
        entry(&report.value, "opencode", "command ivar-plan.md").change,
        Change::Created
    );
}

#[test]
fn second_sync_reports_commands_unchanged_without_rewriting() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    let dir = root.join(".claude/commands");
    let before: Vec<(Utf8PathBuf, Vec<u8>, Option<std::time::SystemTime>)> = commands::catalog()
        .iter()
        .map(|command| {
            let path = dir.join(command.file_name());
            let bytes = fs::read_bytes(&path).unwrap().unwrap();
            let mtime = std::fs::metadata(path.as_std_path())
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            (path, bytes, mtime)
        })
        .collect();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    for (path, before_bytes, before_mtime) in &before {
        assert_eq!(
            fs::read_bytes(path).unwrap().unwrap(),
            *before_bytes,
            "{path}"
        );
        let mtime = std::fs::metadata(path.as_std_path())
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        assert_eq!(&mtime, before_mtime, "{path} must not be rewritten");
    }
    assert_eq!(
        entry(&report.value, "claude-code", "command ivar-plan.md").change,
        Change::Unchanged
    );
}

#[test]
fn sync_repairs_modified_shipped_command_and_preserves_custom_command() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    let custom = root.join(".claude/commands/custom.md");
    fs::write_text(&custom, "mine\n").unwrap();
    fs::write_text(&root.join(".claude/commands/ivar-plan.md"), "changed\n").unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "claude-code", "command ivar-plan.md").change,
        Change::Updated
    );
    assert_eq!(
        fs::read_text(&root.join(".claude/commands/ivar-plan.md"))
            .unwrap()
            .unwrap(),
        embedded("plan")
    );
    assert_eq!(fs::read_text(&custom).unwrap().unwrap(), "mine\n");
}

#[test]
fn sync_removes_only_shipped_commands_for_unavailable_provider() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    let custom = root.join(".opencode/commands/custom.md");
    fs::write_text(&custom, "mine\n").unwrap();

    // Drop OpenCode from the manifest.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "command ivar-plan.md").change,
        Change::Removed
    );
    assert!(
        !fs::exists(&root.join(".opencode/commands/ivar-plan.md")).unwrap(),
        "a dropped provider's shipped commands must be removed"
    );
    assert_eq!(
        fs::read_text(&custom).unwrap().unwrap(),
        "mine\n",
        "the user's command must survive provider removal"
    );
}

/// Deterministic by construction: a regular file occupying the parent path
/// refuses `ensure_dir` regardless of whether the test runs as root —
/// permission bits would not be.
#[test]
fn command_write_failure_warns_and_other_provider_steps_continue() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(&root.join(".opencode"), "not a directory\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(
        !report.is_clean(),
        "a failed command write must not be clean"
    );
    assert!(
        report.value.entries.iter().any(|e| e.surface == "opencode"
            && e.label == "official commands"
            && e.change == Change::Failed),
        "expected a failed opencode commands entry in {:?}",
        report.value.entries
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.subject == "opencode"),
        "expected an opencode warning in {:?}",
        report.warnings
    );
    // The other provider's commands and config completed regardless.
    assert!(fs::is_file(&root.join(".claude/commands/ivar-plan.md")).unwrap());
    assert!(fs::is_file(&root.join("CLAUDE.md")).unwrap());
    // OpenCode's own non-command config still landed.
    assert!(fs::is_file(&root.join("AGENTS.md")).unwrap());
    assert!(fs::is_file(&root.join("opencode.json")).unwrap());
}

// -- root instruction topology --------------------------------------------

#[test]
fn sync_repairs_absent_and_wrong_enabled_symlinks() {
    let (_guard, root) = hall_with_all_providers();
    // Break both aliases: remove CLAUDE.md, point AGENTS.md elsewhere.
    fs::remove_file(&root.join("CLAUDE.md")).unwrap();
    fs::write_text(&root.join("other.md"), "x").unwrap();
    fs::create_symlink(Utf8Path::new("other.md"), &root.join("AGENTS.md")).unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        entry(&report.value, "claude-code", "CLAUDE.md alias").change,
        Change::Created
    );
    assert_eq!(
        entry(&report.value, "opencode", "AGENTS.md alias").change,
        Change::Updated
    );
    assert_eq!(
        fs::read_symlink(&root.join("CLAUDE.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
    assert_eq!(
        fs::read_symlink(&root.join("AGENTS.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
}

#[test]
fn sync_preserves_an_enabled_regular_alias_and_reports_conflict() {
    let (_guard, root) = hall_with_all_providers();
    fs::write_text(&root.join("AGENTS.md"), "legacy, precious\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(!report.is_clean());
    assert_eq!(
        entry(&report.value, "opencode", "AGENTS.md alias").change,
        Change::Failed
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "instructions.adoption_required"),
        "warnings: {:?}",
        report.warnings
    );
    assert_eq!(
        fs::read_text(&root.join("AGENTS.md")).unwrap().unwrap(),
        "legacy, precious\n",
        "an enabled regular alias must be preserved byte for byte"
    );
}

#[test]
fn a_conflict_does_not_abort_repo_mcp_or_command_reconciliation() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        Manifest::read(&layout).unwrap().unwrap().repos().to_vec(),
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    fs::write_text(&root.join("AGENTS.md"), "legacy, precious\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(!report.is_clean());
    assert_eq!(
        entry(&report.value, "opencode", "AGENTS.md alias").change,
        Change::Failed
    );
    // The repo, the MCP config, and the commands still land.
    assert_eq!(
        entry(&report.value, "repo api", "bare clone").change,
        Change::Created
    );
    assert!(fs::is_file(&root.join(".mcp.json")).unwrap());
    assert!(fs::is_file(&root.join(".opencode/commands/ivar-plan.md")).unwrap());
    assert!(fs::is_file(&root.join("CLAUDE.md")).unwrap());
    assert!(fs::is_file(&root.join("HALL.md")).unwrap());
}

#[test]
fn removing_a_provider_by_hand_makes_sync_delete_its_regular_alias() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    // Drop OpenCode from the manifest by hand — the destructive
    // disabled-provider rule now authorises deleting its alias entry.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    fs::remove_file(&root.join("AGENTS.md")).unwrap();
    fs::write_text(&root.join("AGENTS.md"), "regular file\n").unwrap();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(
        entry(&report.value, "opencode", "AGENTS.md alias").change,
        Change::Removed
    );
    assert!(!fs::exists(&root.join("AGENTS.md")).unwrap());
    assert!(
        fs::is_file(&root.join("HALL.md")).unwrap(),
        "the canonical file must always survive"
    );
}

/// `sync` runs after every `git pull`; a healthy second run must leave file
/// mtimes untouched, or every run dirties `git status`.
#[test]
fn repeated_healthy_sync_leaves_file_mtimes_unchanged() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    let paths = [
        root.join("HALL.md"),
        root.join("CLAUDE.md"),
        root.join("AGENTS.md"),
    ];
    let before: Vec<(Utf8PathBuf, Option<std::time::SystemTime>)> = paths
        .iter()
        .map(|path| {
            let mtime = std::fs::metadata(path.as_std_path())
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            (path.clone(), mtime)
        })
        .collect();

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    for (path, before_mtime) in &before {
        let mtime = std::fs::metadata(path.as_std_path())
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        assert_eq!(&mtime, before_mtime, "{path} must not be rewritten");
    }
}

// -- not in a hall ---------------------------------------------------------

#[test]
fn syncing_outside_a_hall_is_blocked_and_points_at_init() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = sync(&ctx, SyncInput::default()).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "hall.not_found");
    assert_eq!(failure.fix_actions[0].command.as_deref(), Some("ivar init"));
}

#[test]
fn sync_works_from_a_subdirectory_of_the_hall() {
    let (_guard, root) = hall_with(&[]);
    let nested = root.join("deep/inside");
    fs::ensure_dir(&nested).unwrap();
    let ctx = Ctx::new(nested);

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert_eq!(report.value.root, root);
}

// -- rendering -------------------------------------------------------------

#[test]
fn the_human_surface_groups_by_surface_and_ends_with_the_counts() {
    let outcome = SyncOutcome {
        root: Utf8PathBuf::from("/hall"),
        entries: vec![
            Entry::new("hall", ".ivar/", Change::Unchanged),
            Entry::new("repo api", "bare clone", Change::Created),
            Entry::new("repo api", "worktree main", Change::Failed).detail("branch not found"),
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Synced /hall\n\
             \n\
             hall:\n\
             \x20 = .ivar/\n\
             \n\
             repo api:\n\
             \x20 + bare clone\n\
             \x20 x worktree main — branch not found\n\
             \n\
             created: 1  updated: 0  removed: 0  unchanged: 1  failed: 1\n"
    );
}

#[test]
fn the_json_surface_carries_every_entry_and_its_change() {
    let outcome = SyncOutcome {
        root: Utf8PathBuf::from("/hall"),
        entries: vec![Entry::new("hall", ".ivar/", Change::Created)],
    };

    let json = serde_json::to_string(&Report::new(outcome)).unwrap();

    assert_eq!(
        json,
        r#"{"root":"/hall","entries":[{"surface":"hall","label":".ivar/","change":"created"}]}"#
    );
}

// -- settings and managed artifact materialisation -------------------------

#[test]
fn sync_materialises_settings_and_artifacts_per_provider() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());

    let report = sync(&ctx, SyncInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(
        root.join(".claude/settings.json").is_file(),
        ".claude/settings.json must be created for Claude Code"
    );
    assert!(
        root.join(".opencode/plugins/ivar.js").is_file(),
        ".opencode/plugins/ivar.js must be created for OpenCode"
    );
    assert!(
        root.join(".omp/hooks/pre/ivar.js").is_file(),
        ".omp/hooks/pre/ivar.js must be created for OMP"
    );
    // The settings file carries ivar's env key.
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_text(&root.join(".claude/settings.json"))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(settings["env"]["IVAR_HALL"], serde_json::json!("acme"));
}

#[test]
fn sync_removes_artifacts_when_provider_is_not_listed() {
    let (_guard, root) = hall_with_all_providers();
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();
    assert!(root.join(".opencode/plugins/ivar.js").is_file());
    assert!(root.join(".omp/hooks/pre/ivar.js").is_file());

    // Drop OpenCode and OMP from the manifest.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    sync(&ctx, SyncInput::default()).unwrap();

    assert!(
        !root.join(".opencode/plugins/ivar.js").exists(),
        "plugin must be removed when OpenCode is not listed"
    );
    assert!(
        !root.join(".omp/hooks/pre/ivar.js").exists(),
        "hook must be removed when OMP is not listed"
    );
    // Claude Code's settings survive.
    assert!(root.join(".claude/settings.json").is_file());
}

/// Protection is not a separate command anyone has to remember: `sync` is the
/// one action that runs over every repo in the hall, so it is where an
/// existing hall acquires protection without being asked.
#[test]
fn sync_protects_the_default_branch_of_every_repo() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "trunk")]);
    let ctx = Ctx::new(root.clone());

    sync(&ctx, SyncInput::default()).unwrap();

    for (repo, branch) in [("api", "main"), ("web", "trunk")] {
        let worktree = root.join(format!(".ivar/repos/{repo}/{branch}"));
        let output = std::process::Command::new("git")
            .args(["-c", "user.name=ivar tests"])
            .args(["-c", "user.email=tests@ivar.invalid"])
            .args(["-c", "commit.gpgsign=false"])
            .args(["commit", "--allow-empty", "-m", "nope"])
            .current_dir(&worktree)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{repo}: a commit on {branch} must be refused"
        );
        assert!(
            stderr.contains(branch),
            "{repo}: the refusal must name {branch}: {stderr}"
        );
    }
}

/// A repo whose setup script installs husky must still end up protected, and
/// the ordering is the whole reason: protection runs after the setup script,
/// so whatever the script wrote to `core.hooksPath` is already there to be
/// overridden for the default worktree.
#[test]
fn protection_outlives_a_setup_script_that_rewrites_hooks_path() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    sync(&ctx, SyncInput::default()).unwrap();

    // Stand in for husky: write the shared config the way `pnpm install` would.
    let bare = root.join(".ivar/repos/api/.bare");
    let elsewhere = root.join("elsewhere");
    crate::infra::fs::ensure_dir(&elsewhere).unwrap();
    crate::test_support::git(&bare, &["config", "core.hooksPath", elsewhere.as_str()]);

    sync(&ctx, SyncInput::default()).unwrap();

    let worktree = root.join(".ivar/repos/api/main");
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(["commit", "--allow-empty", "-m", "nope"])
        .current_dir(&worktree)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a later hook manager must not disarm protection"
    );
}
