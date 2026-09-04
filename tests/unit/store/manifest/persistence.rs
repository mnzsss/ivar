//! Unit tests for `crate::store::manifest::persistence`.
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
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::infra::{fs, json};
use crate::store::manifest::{Providers, Repo, Skills, Targets};
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
        "version": 4,
        "name": "acme",
        "$schema": "https://ivar.run/ivar.schema.json",
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
            assert_eq!(*highest, 4);
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

// -- v1 -> v3 committed migration, through v2 --------------------------------

/// The exact v1 shape ivar wrote before the v2 bump: no `integration` at the
/// top level, no `checks` on any repo.
fn v1_manifest_bytes() -> &'static str {
    r#"{"version":1,"name":"acme","providers":{"available":["claude-code","opencode"],"default":"claude-code"},"repos":[{"name":"api","url":"git@github.com:acme/api.git","default_branch":"main"},{"name":"web","url":"git@github.com:acme/web.git","default_branch":"main"}]}"#
}

/// The exact v2 shape ivar wrote before the v3 bump: `integration` and
/// `checks` present (v1 → v2's own additions), no server carries `oauth`
/// (v3's addition, which v2 could not have had).
fn v2_manifest_bytes() -> &'static str {
    r#"{"version":2,"name":"acme","integration":{"strategy":"squash","via":"local"},"providers":{"available":["claude-code","opencode"],"default":"claude-code"},"repos":[{"name":"api","url":"git@github.com:acme/api.git","default_branch":"main","checks":[]}]}"#
}

/// A v3 file as ivar wrote them when `sse` was still accepted. The transport
/// vocabulary closed to `http`/`local` after v3, so this shape is exactly
/// what a user upgrading across that break has on disk.
fn v3_manifest_bytes() -> &'static str {
    r#"{"version":3,"name":"acme","integration":{"strategy":"squash","via":"local"},"providers":{"available":["claude-code","opencode"],"default":"claude-code"},"repos":[{"name":"api","url":"git@github.com:acme/api.git","default_branch":"main","checks":[]}],"mcp":[{"name":"figma","type":"sse","url":"https://mcp.figma.com/mcp","oauth":{"client_id":"client-123","client_secret_env":"IVAR_MCP_ACME_FIGMA_SECRET"}}]}"#
}

/// The same v3 shape with the transport already migrated by hand: `oauth`
/// present with `client_id` and `client_secret_env` but no `token_url` or
/// `resource` (v4's additions, which a v3 file could not have had).
fn v3_canonical_manifest_bytes() -> &'static str {
    r#"{"version":3,"name":"acme","integration":{"strategy":"squash","via":"local"},"providers":{"available":["claude-code","opencode"],"default":"claude-code"},"repos":[{"name":"api","url":"git@github.com:acme/api.git","default_branch":"main","checks":[]}],"mcp":[{"name":"figma","type":"http","url":"https://mcp.figma.com/mcp","oauth":{"client_id":"client-123","client_secret_env":"IVAR_MCP_ACME_FIGMA_SECRET"}}]}"#
}

#[test]
fn a_v1_manifest_reads_in_memory_as_current_without_rewriting_the_file() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = v1_manifest_bytes();
    fs::write_text(&layout.manifest(), original).unwrap();

    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(manifest.version(), 4);
    assert_eq!(
        manifest.integration(),
        crate::domain::feature::IntegrationPolicy::default()
    );
    assert_eq!(manifest.repos().len(), 2);
    for repo in manifest.repos() {
        assert!(repo.checks().is_empty());
    }
    assert!(manifest.mcp_servers().is_empty());

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "a committed read must never rewrite the file"
    );
}

#[test]
fn a_v2_manifest_reads_in_memory_as_current_without_rewriting_the_file() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = v2_manifest_bytes();
    fs::write_text(&layout.manifest(), original).unwrap();

    let manifest = Manifest::read(&layout).unwrap().unwrap();

    assert_eq!(manifest.version(), 4);
    assert_eq!(manifest.repos().len(), 1);
    assert!(manifest.mcp_servers().is_empty());

    let bytes_after = fs::read_bytes(&layout.manifest()).unwrap().unwrap();
    assert_eq!(
        bytes_after,
        original.as_bytes(),
        "a committed read must never rewrite the file"
    );
}

#[test]
fn migration_plan_available_for_v1_and_v2_and_unreachable_for_v0() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    fs::write_text(&layout.manifest(), v1_manifest_bytes()).unwrap();
    let plan = Manifest::plan(&layout).unwrap().unwrap();
    assert_eq!(plan, MigrationPlan::Available { from: 1, to: 4 });

    fs::write_text(&layout.manifest(), v2_manifest_bytes()).unwrap();
    let plan = Manifest::plan(&layout).unwrap().unwrap();
    assert_eq!(plan, MigrationPlan::Available { from: 2, to: 4 });

    fs::write_text(
        &layout.manifest(),
        r#"{"name":"acme","providers":{"available":["claude-code"],"default":"claude-code"},"repos":[]}"#,
    )
    .unwrap();
    let plan = Manifest::plan(&layout).unwrap().unwrap();
    assert_eq!(plan, MigrationPlan::Unreachable { from: 0, to: 4 });
}

#[test]
fn a_plain_write_refuses_a_v1_file_and_explicit_migrate_writes_canonical_current() {
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

    // The explicit migrate advances the committed file to canonical v3,
    // stepping through v2 on the way.
    let migrated = Manifest::migrate(&layout).unwrap().unwrap();
    assert_eq!(migrated.version(), 4);
    assert_eq!(migrated.repos().len(), 2);

    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    let expected = json::to_canonical_string(&serde_json::json!({
        "version": 4,
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

#[test]
fn a_plain_write_refuses_a_v2_file_and_explicit_migrate_writes_canonical_current() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = v2_manifest_bytes();
    fs::write_text(&layout.manifest(), original).unwrap();

    let error = Manifest::write(&layout, &sample_manifest()).unwrap_err();
    assert!(matches!(
        error,
        Error::Store(versioned::Error::CommittedRefusesImplicitUpgrade { .. })
    ));

    // v2 -> v3 adds nothing to the data — the migration exists purely to
    // advance the stamped version, and this hall's own repo (with no `mcp`
    // array at all) proves that round-trips exactly as it did before.
    let migrated = Manifest::migrate(&layout).unwrap().unwrap();
    assert_eq!(migrated.version(), 4);
    assert!(migrated.mcp_servers().is_empty());

    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    let expected = json::to_canonical_string(&serde_json::json!({
        "version": 4,
        "name": "acme",
        "integration": { "strategy": "squash", "via": "local" },
        "providers": { "available": ["claude-code", "opencode"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": "git@github.com:acme/api.git", "default_branch": "main" }
        ],
    }))
    .unwrap();
    assert_eq!(on_disk, expected);
}

/// `C-BREAKING-CHANGE`: `sse` is not a deprecated alias, so a v3 file that
/// used it cannot migrate silently — the transport is a manual edit. The
/// refusal must name the entry and the canonical replacement rather than
/// failing as a generic parse error.
#[test]
fn a_v3_manifest_using_a_retired_transport_refuses_to_migrate_and_names_the_replacement() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    fs::write_text(&layout.manifest(), v3_manifest_bytes()).unwrap();

    let error = Manifest::migrate(&layout).unwrap_err();
    match &error {
        Error::InvalidMcpType { name, transport } => {
            assert_eq!(name, "figma");
            assert_eq!(transport, "sse");
        }
        other => panic!("expected InvalidMcpType, got: {other:?}"),
    }

    let failure = crate::error::Failure::from(error);
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|action| action.what.contains("`http`")),
        "the fix must name the canonical replacement: {failure:?}"
    );

    // The file is left exactly as it was: a refused migration never rewrites.
    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    assert_eq!(on_disk, v3_manifest_bytes());
}

/// A v3 file whose MCP entries already use the canonical vocabulary migrates
/// to v4 with its `oauth` fields intact, and without fabricating a
/// `token_url` or `resource` that the v3 file never carried.
#[test]
fn a_v3_manifest_with_oauth_migrates_to_v4_with_fields_intact_and_no_fabricated_token_url() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);
    let original = v3_canonical_manifest_bytes();
    fs::write_text(&layout.manifest(), original).unwrap();

    let error = Manifest::write(&layout, &sample_manifest()).unwrap_err();
    assert!(matches!(
        error,
        Error::Store(versioned::Error::CommittedRefusesImplicitUpgrade { .. })
    ));

    let migrated = Manifest::migrate(&layout).unwrap().unwrap();
    assert_eq!(migrated.version(), 4);
    let servers = migrated.mcp_servers();
    assert_eq!(servers.len(), 1);
    let server = &servers[0];
    assert_eq!(server.name, "figma");
    let oauth = server.oauth.as_ref().expect("oauth must remain present");
    assert_eq!(oauth.client_id, "client-123");
    assert_eq!(
        oauth.client_secret_env.as_deref(),
        Some("IVAR_MCP_ACME_FIGMA_SECRET")
    );
    assert_eq!(oauth.token_url, None);
    assert_eq!(oauth.resource, None);

    let on_disk = fs::read_text(&layout.manifest()).unwrap().unwrap();
    let expected = json::to_canonical_string(&serde_json::json!({
        "version": 4,
        "name": "acme",
        "integration": { "strategy": "squash", "via": "local" },
        "providers": { "available": ["claude-code", "opencode"], "default": "claude-code" },
        "repos": [
            { "name": "api", "url": "git@github.com:acme/api.git", "default_branch": "main" }
        ],
        "mcp": [
            {
                "name": "figma",
                "type": "http",
                "url": "https://mcp.figma.com/mcp",
                "oauth": {
                    "client_id": "client-123",
                    "client_secret_env": "IVAR_MCP_ACME_FIGMA_SECRET"
                }
            }
        ]
    }))
    .unwrap();
    assert_eq!(on_disk, expected);
}

#[test]
fn migration_plan_reports_available_from_v3_to_v4() {
    let (_dir, root) = utf8_temp_dir();
    let layout = Layout::at(root);

    fs::write_text(&layout.manifest(), v3_manifest_bytes()).unwrap();
    let plan = Manifest::plan(&layout).unwrap().unwrap();
    assert_eq!(plan, MigrationPlan::Available { from: 3, to: 4 });
}
