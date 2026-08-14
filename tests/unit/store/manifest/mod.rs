#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::mcp::McpServerDef;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Failure, Status};
use crate::infra::{fs, json};
use crate::store::layout::Layout;
use crate::store::versioned;
use crate::test_support::utf8_temp_dir;

fn sample_manifest() -> Manifest {
    Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            "git@github.com:acme/api.git",
            BranchName::new("main").unwrap(),
        )],
        Some(Skills::new(Targets::new(true, true))),
    )
    .unwrap()
}

// -- round-trip: write, read back, exact bytes on disk -------------------

#[test]
fn write_then_read_round_trips_and_writes_canonical_bytes() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let manifest = sample_manifest();

    Manifest::write(&layout, &manifest).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "version": 2,
        "name": "acme",
        "integration": { "strategy": "squash", "via": "local" },
        "providers": { "available": ["claude-code", "opencode"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": "git@github.com:acme/api.git", "default_branch": "main" }
        ],
        "skills": { "targets": { "claude": true, "opencode": true } },
    }))
    .unwrap();
    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    assert_eq!(
        on_disk, expected,
        "write must produce the canonical byte format"
    );

    let read_back = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(read_back, manifest);
}

#[test]
fn manifest_without_skills_omits_the_key_and_still_round_trips() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();

    Manifest::write(&layout, &manifest).unwrap();

    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    assert!(!on_disk.contains("skills"));
    assert_eq!(Manifest::read(&layout).unwrap(), Some(manifest));
}

// -- absent is Ok(None); unparseable is a hard error ----------------------

#[test]
fn absent_file_reads_as_ok_none() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    assert_eq!(Manifest::read(&layout).unwrap(), None);
}

#[test]
fn present_but_unparseable_file_is_a_hard_error() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(&layout.manifest(), "{ not json").unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    assert!(matches!(error, Error::Store(versioned::Error::Json(_))));
}

// -- unknown key is a hard error naming the key ---------------------------

#[test]
fn unknown_key_is_rejected_naming_the_key() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(
            &layout.manifest(),
            r#"{"version":1,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"nickname":"oops"}"#,
        )
        .unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("nickname"), "message was: {message}");
}

// -- the invariants, violated, rejected on read ---------------------------

#[test]
fn default_provider_not_in_available_is_rejected_on_read() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(
            &layout.manifest(),
            r#"{"version":1,"name":"acme","providers":{"available":["claude-code"],"default":"opencode"},"repos":[]}"#,
        )
        .unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    match error {
        Error::DefaultProviderNotAvailable { default, available } => {
            assert_eq!(default, Provider::OpenCode);
            assert_eq!(available, vec![Provider::ClaudeCode]);
        }
        other => panic!("expected DefaultProviderNotAvailable, got {other:?}"),
    }
}

#[test]
fn empty_available_providers_is_rejected_on_read() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(
            &layout.manifest(),
            r#"{"version":1,"name":"acme","providers":{"available":[],"default":"claude-code"},"repos":[]}"#,
        )
        .unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    assert!(matches!(error, Error::NoAvailableProviders));
}

#[test]
fn duplicate_repo_names_are_rejected_on_read() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(
            &layout.manifest(),
            r#"{"version":1,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[
                {"name":"api","url":"a","default_branch":"main"},
                {"name":"api","url":"b","default_branch":"main"}
            ]}"#,
        )
        .unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    match error {
        Error::DuplicateRepoName { name } => assert_eq!(name.as_str(), "api"),
        other => panic!("expected DuplicateRepoName, got {other:?}"),
    }
}

#[test]
fn an_empty_repo_url_is_rejected_on_read() {
    // Not path safety — a remote URL never becomes a path. This is the
    // difference between naming the offending repo and letting `ivar sync`
    // hand the user a bare `git clone` error.
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(
            &layout.manifest(),
            r#"{"version":1,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[
                {"name":"api","url":"   ","default_branch":"main"}
            ]}"#,
        )
        .unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    match error {
        Error::EmptyRepoUrl { name } => assert_eq!(name.as_str(), "api"),
        other => panic!("expected EmptyRepoUrl, got {other:?}"),
    }
}

// -- a hand-edited traversal in a repo name is rejected at deserialize ----

#[test]
fn repo_name_traversal_is_rejected_when_deserialized() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(
            &layout.manifest(),
            r#"{"version":1,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[
                {"name":"../etc","url":"a","default_branch":"main"}
            ]}"#,
        )
        .unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    assert!(matches!(
        error,
        Error::Store(versioned::Error::Deserialize { .. })
    ));
}

// -- no version field is rejected, not adopted as v0 ----------------------

#[test]
fn missing_version_field_is_rejected_not_adopted_as_v0() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[]}"#;
    fs::write_text(&layout.manifest(), original).unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    assert!(matches!(error, Error::MissingVersion { .. }));

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "refusing an unversioned file must not touch it"
    );
}

// -- a version newer than current is refused, file untouched -------------

#[test]
fn version_newer_than_current_is_refused_and_the_file_is_untouched() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = r#"{"version":99,"name":"from-the-future","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[]}"#;
    fs::write_text(&layout.manifest(), original).unwrap();

    let error = Manifest::read(&layout).unwrap_err();
    match &error {
        Error::Store(versioned::Error::TooNew { found, highest, .. }) => {
            assert_eq!(*found, 99);
            assert_eq!(*highest, 2);
        }
        other => panic!("expected TooNew, got {other:?}"),
    }

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(bytes_after, original.as_bytes());
}

// -- Policy::Committed: a plain read never rewrites the file -------------

#[test]
fn committed_policy_read_never_rewrites_the_file() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    // Valid v1 data, deliberately not in canonical form (unsorted keys, no
    // trailing newline). A plain `read` must never rewrite this to
    // canonical form on its own — only an explicit `write` does that.
    let original = r#"{"repos":[],"name":"acme","version":1,"providers":{"default":"claude-code","available":["claude-code"]}}"#;
    fs::write_text(&layout.manifest(), original).unwrap();

    let manifest = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(manifest.name().as_str(), "acme");

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "read must never rewrite a committed file"
    );
}

#[test]
fn write_refuses_when_on_disk_is_older_than_current_and_leaves_it_untouched() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[]}"#;
    fs::write_text(&layout.manifest(), original).unwrap();

    let error = Manifest::write(&layout, &sample_manifest()).unwrap_err();
    assert!(matches!(
        error,
        Error::Store(versioned::Error::CommittedRefusesImplicitUpgrade { .. })
    ));

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(bytes_after, original.as_bytes());
}

// -- Manifest::new validates by construction ------------------------------

#[test]
fn new_rejects_default_provider_not_in_available() {
    let error = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::OpenCode),
        vec![],
        None,
    )
    .unwrap_err();
    assert!(matches!(error, Error::DefaultProviderNotAvailable { .. }));
}

#[test]
fn new_rejects_empty_available_providers() {
    let error = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap_err();
    assert!(matches!(error, Error::NoAvailableProviders));
}

#[test]
fn new_rejects_duplicate_repo_names() {
    let repo = |url: &str| {
        Repo::new(
            RepoName::new("api").unwrap(),
            url,
            BranchName::new("main").unwrap(),
        )
    };
    let error = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![repo("a"), repo("b")],
        None,
    )
    .unwrap_err();
    assert!(matches!(error, Error::DuplicateRepoName { .. }));
}

#[test]
fn new_accepts_a_well_formed_manifest() {
    let manifest = sample_manifest();
    assert_eq!(manifest.version(), 2);
    assert_eq!(manifest.name().as_str(), "acme");
    assert_eq!(
        manifest.providers().available(),
        [Provider::ClaudeCode, Provider::OpenCode]
    );
    assert_eq!(
        manifest.providers().default_provider(),
        Provider::ClaudeCode
    );
    assert_eq!(manifest.repos().len(), 1);
    assert!(manifest.skills().is_some());
}

// -- accessors -------------------------------------------------------------

#[test]
fn repo_accessors_return_the_constructed_values() {
    let repo = Repo::new(
        RepoName::new("api").unwrap(),
        "git@github.com:acme/api.git",
        BranchName::new("main").unwrap(),
    );
    assert_eq!(repo.name().as_str(), "api");
    assert_eq!(repo.url(), "git@github.com:acme/api.git");
    assert_eq!(repo.default_branch().as_str(), "main");
}

// -- mutation: with_repo_added / with_repo_removed -------------------------

fn empty_manifest() -> Manifest {
    Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap()
}

fn repo(name: &str, url: &str) -> Repo {
    Repo::new(
        RepoName::new(name).unwrap(),
        url,
        BranchName::new("main").unwrap(),
    )
}

#[test]
fn with_repo_added_appends_and_keeps_the_original_untouched() {
    let original = empty_manifest();

    let updated = original.with_repo_added(repo("api", "url-a")).unwrap();

    assert_eq!(updated.repos().len(), 1);
    assert_eq!(updated.repos()[0].name().as_str(), "api");
    // The original is not mutated — callers rewrite the file from the
    // returned value, so this is what makes add/remove transactional.
    assert!(original.repos().is_empty());
}

#[test]
fn with_repo_added_rejects_a_duplicate_name() {
    let manifest = empty_manifest()
        .with_repo_added(repo("api", "url-a"))
        .unwrap();

    let error = manifest.with_repo_added(repo("api", "url-b")).unwrap_err();

    assert!(matches!(error, Error::DuplicateRepoName { .. }));
}

#[test]
fn with_repo_removed_drops_the_named_repo_and_keeps_the_rest() {
    let manifest = empty_manifest()
        .with_repo_added(repo("api", "url-a"))
        .unwrap()
        .with_repo_added(repo("web", "url-b"))
        .unwrap();

    let updated = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();

    assert_eq!(updated.repos().len(), 1);
    assert_eq!(updated.repos()[0].name().as_str(), "web");
    // Removing never touches the filesystem — that is `ivar cleanup`'s job.
    assert_eq!(manifest.repos().len(), 2);
}

#[test]
fn with_repo_removed_rejects_a_name_that_is_not_present() {
    let manifest = empty_manifest();

    let error = manifest
        .with_repo_removed(&RepoName::new("ghost").unwrap())
        .unwrap_err();

    assert!(matches!(error, Error::RepoNotFound { .. }));
}

#[test]
fn repo_not_found_failure_names_the_repo_and_a_safe_fix() {
    let failure: Failure = Error::RepoNotFound {
        name: RepoName::new("ghost").unwrap(),
    }
    .into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.repo_not_found");
    assert!(failure.what.contains("ghost"));
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn targets_accessors_return_the_constructed_values() {
    let targets = Targets::new(true, false);
    assert!(targets.claude());
    assert!(!targets.opencode());
}

#[test]
fn skills_accessor_returns_the_constructed_targets() {
    let skills = Skills::new(Targets::new(true, true));
    assert!(skills.targets().claude());
    assert!(skills.targets().opencode());
}

// -- Error -> Failure: every variant has its own code and fix action -----

#[test]
fn missing_version_failure_names_the_path_and_a_safe_fix() {
    let failure: Failure = Error::MissingVersion {
        path: Utf8PathBuf::from("/hall/ivar.json"),
    }
    .into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.missing_version");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn default_provider_not_available_failure_names_the_offending_value() {
    let failure: Failure = Error::DefaultProviderNotAvailable {
        default: Provider::OpenCode,
        available: vec![Provider::ClaudeCode],
    }
    .into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.default_provider_not_available");
    assert!(failure.what.contains("opencode"));
    assert!(failure.actual.as_deref().unwrap().contains("opencode"));
    assert!(failure.fix_actions[0].what.contains("claude-code"));
}

#[test]
fn no_available_providers_failure_has_a_safe_fix() {
    let failure: Failure = Error::NoAvailableProviders.into();
    assert_eq!(failure.code, "manifest.no_available_providers");
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn duplicate_repo_name_failure_names_the_offending_repo() {
    let failure: Failure = Error::DuplicateRepoName {
        name: RepoName::new("api").unwrap(),
    }
    .into();
    assert_eq!(failure.code, "manifest.duplicate_repo_name");
    assert!(failure.what.contains("api"));
    assert!(failure.fix_actions[0].what.contains("api"));
}

#[test]
fn store_error_delegates_its_failure_conversion() {
    let failure: Failure = Error::Store(versioned::Error::TooNew {
        path: Utf8PathBuf::from("/hall/ivar.json"),
        found: 2,
        highest: 1,
    })
    .into();
    assert_eq!(failure.code, "store.version_too_new");
}

// -- MCP server definitions ----------------------------------------------

fn mcp_server(name: &str) -> McpServerDef {
    McpServerDef::new(name, "stdio").command("npx")
}

#[test]
fn a_manifest_without_mcp_servers_reports_an_empty_slice() {
    let manifest = empty_manifest();

    assert!(manifest.mcp_servers().is_empty());
}

#[test]
fn with_mcp_servers_carries_the_definitions_into_the_manifest() {
    let manifest = empty_manifest()
        .with_mcp_servers(vec![mcp_server("docs"), mcp_server("sentry")])
        .unwrap();

    assert_eq!(manifest.mcp_servers().len(), 2);
    assert_eq!(manifest.mcp_servers()[0].name, "docs");
    assert_eq!(manifest.mcp_servers()[1].name, "sentry");
}

#[test]
fn with_mcp_servers_rejects_duplicate_names() {
    let error = empty_manifest()
        .with_mcp_servers(vec![mcp_server("docs"), mcp_server("docs")])
        .unwrap_err();

    match error {
        Error::DuplicateMcpServerName { name } => assert_eq!(name, "docs"),
        other => panic!("expected DuplicateMcpServerName, got {other:?}"),
    }
}

#[test]
fn an_empty_server_list_is_stored_as_absent_and_omits_the_key() {
    let manifest = empty_manifest()
        .with_mcp_servers(vec![])
        .unwrap()
        .with_mcp_servers(vec![mcp_server("docs")])
        .unwrap()
        .with_mcp_servers(vec![])
        .unwrap();

    assert!(manifest.mcp_servers().is_empty());

    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    Manifest::write(&layout, &manifest).unwrap();
    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    assert!(
        !on_disk.contains("mcp"),
        "an absent mcp list must stay off disk: {on_disk}"
    );
}

#[test]
fn mcp_servers_round_trip_through_write_and_read() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let manifest = empty_manifest()
        .with_mcp_servers(vec![mcp_server("docs")])
        .unwrap();

    Manifest::write(&layout, &manifest).unwrap();

    let read_back = Manifest::read(&layout).unwrap().unwrap();
    assert_eq!(read_back, manifest);
    assert_eq!(read_back.mcp_servers()[0].name, "docs");
}

#[test]
fn duplicate_mcp_server_failure_names_the_offending_name() {
    let failure: Failure = Error::DuplicateMcpServerName {
        name: "docs".to_owned(),
    }
    .into();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "manifest.duplicate_mcp_server_name");
    assert!(failure.what.contains("docs"));
    assert!(failure.fix_actions[0].safe);
}

// -- v2: hall integration defaults and ordered repo checks ------------------

#[test]
fn the_v1_constructor_calls_keep_their_compatibility_defaults() {
    let repo = Repo::new(
        RepoName::new("api").unwrap(),
        "git@github.com:acme/api.git",
        BranchName::new("main").unwrap(),
    );
    assert!(repo.checks().is_empty());

    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![repo],
        None,
    )
    .unwrap();
    assert_eq!(
        manifest.integration(),
        crate::domain::feature::IntegrationPolicy::default()
    );
}

#[test]
fn builders_configure_checks_and_hall_integration() {
    let repo = Repo::new(
        RepoName::new("api").unwrap(),
        "git@github.com:acme/api.git",
        BranchName::new("main").unwrap(),
    )
    .with_checks(vec![
        "cargo fmt --check".to_owned(),
        "cargo test --all-features".to_owned(),
    ]);
    assert_eq!(
        repo.checks(),
        ["cargo fmt --check", "cargo test --all-features"]
    );

    let manifest = sample_manifest().with_integration(
        crate::domain::feature::IntegrationPolicy {
            via: crate::domain::feature::IntegrationVia::Pr,
            strategy: crate::domain::feature::IntegrationStrategy::Rebase,
        },
    );
    assert_eq!(
        manifest.integration(),
        crate::domain::feature::IntegrationPolicy {
            via: crate::domain::feature::IntegrationVia::Pr,
            strategy: crate::domain::feature::IntegrationStrategy::Rebase,
        }
    );
}

#[test]
fn repo_add_remove_provider_add_and_mcp_preserve_both_v2_fields() {
    let manifest = sample_manifest()
        .with_integration(crate::domain::feature::IntegrationPolicy {
            via: crate::domain::feature::IntegrationVia::Pr,
            strategy: crate::domain::feature::IntegrationStrategy::Merge,
        });
    let manifest = manifest.with_repo_added(
        Repo::new(
            RepoName::new("web").unwrap(),
            "git@github.com:acme/web.git",
            BranchName::new("main").unwrap(),
        )
        .with_checks(vec!["npm test".to_owned()]),
    ).unwrap();
    assert_eq!(manifest.integration().via, crate::domain::feature::IntegrationVia::Pr);
    assert_eq!(manifest.repos()[1].checks(), ["npm test"]);

    let manifest = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();
    assert_eq!(manifest.repos().len(), 1);
    assert_eq!(manifest.integration().strategy, crate::domain::feature::IntegrationStrategy::Merge);

    let manifest = manifest
        .with_providers(Providers::new(vec![Provider::OpenCode], Provider::OpenCode))
        .unwrap();
    assert_eq!(manifest.integration().via, crate::domain::feature::IntegrationVia::Pr);
    assert_eq!(manifest.repos()[0].checks(), ["npm test"]);

    let manifest = manifest
        .with_mcp_servers(vec![mcp_server("docs")])
        .unwrap();
    assert_eq!(manifest.integration().strategy, crate::domain::feature::IntegrationStrategy::Merge);
    assert_eq!(manifest.mcp_servers().len(), 1);
}

#[test]
fn a_blank_repo_check_is_refused_on_build() {
    let error = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            "git@github.com:acme/api.git",
            BranchName::new("main").unwrap(),
        )
        .with_checks(vec!["   ".to_owned()])],
        None,
    )
    .unwrap_err();
    match error {
        Error::EmptyRepoCheck { name, index } => {
            assert_eq!(name.as_str(), "api");
            assert_eq!(index, 0);
        }
        other => panic!("expected EmptyRepoCheck, got {other:?}"),
    }
}

// -- v1 -> v2 committed migration -------------------------------------------

/// The exact v1 shape ivar wrote before the v2 bump: no `integration` at the
/// top level, no `checks` on any repo.
fn v1_manifest_bytes() -> &'static str {
    r#"{"version":1,"name":"acme","providers":{"available":["claude-code","opencode"],"default":"claude-code"},"repos":[{"name":"api","url":"git@github.com:acme/api.git","default_branch":"main"},{"name":"web","url":"git@github.com:acme/web.git","default_branch":"main"}]}"#
}

#[test]
fn a_v1_manifest_reads_in_memory_as_v2_without_rewriting_the_file() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = v1_manifest_bytes();
    fs::write_text(&layout.manifest(), original).unwrap();

    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(manifest.version(), 2);
    assert_eq!(
        manifest.integration(),
        crate::domain::feature::IntegrationPolicy::default()
    );
    assert_eq!(manifest.repos().len(), 2);
    for repo in manifest.repos() {
        assert!(repo.checks().is_empty());
    }

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "a committed read must never rewrite the file"
    );
}

#[test]
fn migration_plan_available_for_v1_and_unreachable_for_v0() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    fs::write_text(&layout.manifest(), v1_manifest_bytes()).unwrap();
    let plan = Manifest::plan(&layout).unwrap().unwrap();
    assert_eq!(
        plan,
        MigrationPlan::Available { from: 1, to: 2 }
    );

    fs::write_text(
        &layout.manifest(),
        r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[]}"#,
    )
    .unwrap();
    let plan = Manifest::plan(&layout).unwrap().unwrap();
    assert_eq!(
        plan,
        MigrationPlan::Unreachable { from: 0, to: 2 }
    );
}

#[test]
fn a_plain_write_refuses_a_v1_file_and_explicit_migrate_writes_canonical_v2() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = v1_manifest_bytes();
    fs::write_text(&layout.manifest(), original).unwrap();

    // A plain write refuses while the on-disk file is older than current.
    let error = Manifest::write(&layout, &sample_manifest()).unwrap_err();
    assert!(matches!(
        error,
        Error::Store(versioned::Error::CommittedRefusesImplicitUpgrade { .. })
    ));

    // The explicit migrate advances the committed file to canonical v2.
    let migrated = Manifest::migrate(&layout).unwrap().unwrap();
    assert_eq!(migrated.version(), 2);
    assert_eq!(migrated.repos().len(), 2);

    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    let expected = json::to_canonical_string(&serde_json::json!({
        "version": 2,
        "name": "acme",
        "integration": { "strategy": "squash", "via": "local" },
        "providers": { "available": ["claude-code", "opencode"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": "git@github.com:acme/api.git", "default_branch": "main" },
            { "name": "web", "url": "git@github.com:acme/web.git", "default_branch": "main" }
        ],
    }))
    .unwrap();
    assert_eq!(on_disk, expected);
}
