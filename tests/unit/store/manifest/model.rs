//! Unit tests for `crate::store::manifest::model`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::error::{Failure, Status};
use crate::infra::fs;
use crate::store::layout::Layout;
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
    assert_eq!(manifest.version(), 4);
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

// -- MCP server definitions ----------------------------------------------

fn mcp_server(name: &str) -> McpServerDef {
    McpServerDef::new(name, "local").command("npx")
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
fn invalid_mcp_configurations_are_rejected_on_read() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    // Test cases for invalid MCP configurations:
    // 1. http without url
    // 2. http with empty url
    // 3. http carrying command/args/env
    // 4. local without command
    // 5. local with empty command
    // 6. local carrying url
    // 7. args without command

    let invalid_configs = vec![
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"http-no-url","type":"http"}]}"#,
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"http-empty-url","type":"http","url":"  "}]}"#,
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"http-with-cmd","type":"http","url":"http://x","command":"ls"}]}"#,
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"local-no-cmd","type":"local"}]}"#,
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"local-empty-cmd","type":"local","command":"  "}]}"#,
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"local-with-url","type":"local","command":"ls","url":"http://x"}]}"#,
        r#"{"version":3,"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[],"integration":{"via":"local","strategy":"squash"},"mcp":[{"name":"local-args-no-cmd","type":"local","args":["-y"]}]}"#,
    ];

    for json in invalid_configs {
        fs::write_text(&layout.manifest(), json).unwrap();
        let error = Manifest::read(&layout).unwrap_err();
        match error {
            Error::InvalidMcpServerDefinition { .. } => {} // Correct
            other => panic!("expected InvalidMcpServerDefinition, got {other:?}"),
        }
    }
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

    let manifest = sample_manifest().with_integration(crate::domain::feature::IntegrationPolicy {
        via: crate::domain::feature::IntegrationVia::Pr,
        strategy: crate::domain::feature::IntegrationStrategy::Rebase,
    });
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
    let manifest = sample_manifest().with_integration(crate::domain::feature::IntegrationPolicy {
        via: crate::domain::feature::IntegrationVia::Pr,
        strategy: crate::domain::feature::IntegrationStrategy::Merge,
    });
    let manifest = manifest
        .with_repo_added(
            Repo::new(
                RepoName::new("web").unwrap(),
                "git@github.com:acme/web.git",
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["npm test".to_owned()]),
        )
        .unwrap();
    assert_eq!(
        manifest.integration().via,
        crate::domain::feature::IntegrationVia::Pr
    );
    assert_eq!(manifest.repos()[1].checks(), ["npm test"]);

    let manifest = manifest
        .with_repo_removed(&RepoName::new("api").unwrap())
        .unwrap();
    assert_eq!(manifest.repos().len(), 1);
    assert_eq!(
        manifest.integration().strategy,
        crate::domain::feature::IntegrationStrategy::Merge
    );

    let manifest = manifest
        .with_providers(Providers::new(vec![Provider::OpenCode], Provider::OpenCode))
        .unwrap();
    assert_eq!(
        manifest.integration().via,
        crate::domain::feature::IntegrationVia::Pr
    );
    assert_eq!(manifest.repos()[0].checks(), ["npm test"]);

    let manifest = manifest.with_mcp_servers(vec![mcp_server("docs")]).unwrap();
    assert_eq!(
        manifest.integration().strategy,
        crate::domain::feature::IntegrationStrategy::Merge
    );
    assert_eq!(manifest.mcp_servers().len(), 1);
}

#[test]
fn a_blank_repo_check_is_refused_on_build() {
    let error = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                "git@github.com:acme/api.git",
                BranchName::new("main").unwrap(),
            )
            .with_checks(vec!["   ".to_owned()]),
        ],
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
