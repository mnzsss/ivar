#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::test_support::utf8_temp_dir;

fn hall() -> HallName {
    HallName::new("acme").unwrap()
}

fn repo(name: &str) -> RepoName {
    RepoName::new(name).unwrap()
}

// -- build_block ----------------------------------------------------------

#[test]
fn the_block_is_delimited_by_the_markers_and_names_the_hall() {
    let block = build_block(&hall(), &[repo("api")]);

    assert!(block.starts_with(MANAGED_START));
    assert!(block.ends_with(MANAGED_END));
    assert!(block.contains("# acme"));
}

#[test]
fn repos_are_listed_in_the_order_given() {
    let block = build_block(&hall(), &[repo("web"), repo("api")]);

    let web = block.find("`web`").unwrap();
    let api = block.find("`api`").unwrap();
    assert!(web < api, "manifest order must survive into the block");
}

#[test]
fn a_hall_with_no_repos_says_how_to_add_one() {
    let block = build_block(&hall(), &[]);

    assert!(block.contains("ivar.json"));
    assert!(block.contains("ivar sync"));
}

/// [`materialise`] decides "unchanged" by comparing bytes, so the builder
/// has to be a function of its arguments and nothing else.
#[test]
fn building_the_same_block_twice_produces_identical_bytes() {
    let first = build_block(&hall(), &[repo("api")]);
    let second = build_block(&hall(), &[repo("api")]);

    assert_eq!(first, second);
}

// -- materialise: the three placement cases -------------------------------

#[test]
fn an_absent_file_is_created_holding_only_the_block() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Created);

    assert_eq!(fs::read_text(&path).unwrap().unwrap(), format!("{block}\n"));
}

#[test]
fn an_existing_block_is_replaced_in_place_leaving_the_users_text_alone() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    let first = build_block(&hall(), &[repo("api")]);
    fs::write_text(
        &path,
        &format!("# House rules\n\n{first}\n\nNever force-push.\n"),
    )
    .unwrap();

    let second = build_block(&hall(), &[repo("api"), repo("web")]);
    assert_eq!(materialise(&path, &second).unwrap(), Change::Updated);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert!(content.starts_with("# House rules\n"));
    assert!(content.ends_with("Never force-push.\n"));
    assert!(content.contains("`web`"));
    assert_eq!(content.matches(MANAGED_START).count(), 1);
}

#[test]
fn a_file_with_no_markers_keeps_every_byte_and_gains_the_block_on_top() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    fs::write_text(&path, "# House rules\n\nNever force-push.\n").unwrap();
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Updated);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert!(content.starts_with(MANAGED_START));
    assert!(content.contains("# House rules"));
    assert!(content.contains("Never force-push."));
}

/// `ivar sync` runs after every `git pull`. A version that rewrote the file
/// each time would put a spurious modification in `git status` on every
/// run.
#[test]
fn materialising_the_same_block_twice_reports_unchanged_and_does_not_rewrite() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Created);
    let after_first = fs::read_bytes(&path).unwrap().unwrap();

    assert_eq!(materialise(&path, &block).unwrap(), Change::Unchanged);
    assert_eq!(fs::read_bytes(&path).unwrap().unwrap(), after_first);
}

/// An end marker before a start marker is not a block to splice — treating
/// it as one would replace the region *between* them, which is the user's
/// text, with the block.
#[test]
fn reversed_markers_are_treated_as_no_block_rather_than_spliced() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("CLAUDE.md");
    fs::write_text(
        &path,
        &format!("{MANAGED_END}\nprecious user text\n{MANAGED_START}\n"),
    )
    .unwrap();
    let block = build_block(&hall(), &[repo("api")]);

    assert_eq!(materialise(&path, &block).unwrap(), Change::Updated);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert!(
        content.contains("precious user text"),
        "the user's text must survive: {content}"
    );
}

// -- remove ---------------------------------------------------------------

#[test]
fn removing_from_a_file_that_held_only_the_block_deletes_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("AGENTS.md");
    let block = build_block(&hall(), &[repo("api")]);
    materialise(&path, &block).unwrap();

    assert_eq!(remove(&path).unwrap(), Change::Removed);
    assert!(!fs::exists(&path).unwrap());
}

#[test]
fn removing_from_a_file_the_user_wrote_in_keeps_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("AGENTS.md");
    let block = build_block(&hall(), &[repo("api")]);
    fs::write_text(&path, &format!("{block}\n\n# House rules\n")).unwrap();

    assert_eq!(remove(&path).unwrap(), Change::Removed);

    let content = fs::read_text(&path).unwrap().unwrap();
    assert_eq!(content, "# House rules\n");
}

#[test]
fn removing_when_there_is_nothing_to_remove_is_unchanged() {
    let (_guard, dir) = utf8_temp_dir();
    let absent = dir.join("AGENTS.md");
    assert_eq!(remove(&absent).unwrap(), Change::Unchanged);

    let untouched = dir.join("CLAUDE.md");
    fs::write_text(&untouched, "# House rules\n").unwrap();
    assert_eq!(remove(&untouched).unwrap(), Change::Unchanged);
    assert_eq!(
        fs::read_text(&untouched).unwrap().unwrap(),
        "# House rules\n"
    );
}

// -- Error -> Failure ------------------------------------------------------

#[test]
fn an_io_error_keeps_the_fs_layers_code_and_names_the_file() {
    let (_guard, dir) = utf8_temp_dir();
    // A directory where a file is expected: reading it fails at the fs
    // layer, which is the mechanical cause this module wraps.
    let path = dir.join("CLAUDE.md");
    std::fs::create_dir_all(&path).unwrap();

    let error = materialise(&path, "block").expect_err("cannot read a directory as text");
    let failure: Failure = error.into();

    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "harness.check_instruction_file"),
        "expected the file-naming fix action, got {:?}",
        failure.fix_actions
    );
}

// -- materialise_mcp: Claude Code ----------------------------------------

/// The v1 case the sync step starts from: no servers declared, and still a
/// valid — empty — config at the hall root, so walk-up discovery finds it.
#[test]
fn an_empty_server_list_materialises_a_valid_empty_config() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");

    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &[]).unwrap(),
        Change::Created
    );

    let expected = json::to_canonical_string(&serde_json::json!({ "mcpServers": {} })).unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

#[test]
fn a_stdio_server_is_serialised_with_command_args_and_env() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    let servers = vec![
        McpServerDef::new("docs", "stdio")
            .command("npx")
            .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
            .env(std::collections::BTreeMap::from([(
                "TOKEN".to_owned(),
                "{env:TOKEN}".to_owned(),
            )])),
    ];

    materialise_mcp(&path, Provider::ClaudeCode, &servers).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "mcpServers": {
            "docs": {
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@acme/docs-mcp"],
                "env": { "TOKEN": "{env:TOKEN}" },
            }
        }
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

/// `ivar sync` runs after every `git pull`; a second run must touch nothing.
#[test]
fn materialising_mcp_twice_is_unchanged_and_does_not_rewrite() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    let servers = vec![McpServerDef::new("docs", "stdio").command("npx")];

    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &servers).unwrap(),
        Change::Created
    );
    let after_first = fs::read_bytes(&path).unwrap().unwrap();

    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &servers).unwrap(),
        Change::Unchanged
    );
    assert_eq!(fs::read_bytes(&path).unwrap().unwrap(), after_first);
}

#[test]
fn changed_servers_rewrite_the_config() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    materialise_mcp(&path, Provider::ClaudeCode, &[]).unwrap();

    let with_server = vec![McpServerDef::new("docs", "stdio").command("npx")];
    assert_eq!(
        materialise_mcp(&path, Provider::ClaudeCode, &with_server).unwrap(),
        Change::Updated
    );

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(on_disk.contains("\"docs\""), "was: {on_disk}");
}

// -- materialise_mcp: OpenCode -------------------------------------------

#[test]
fn opencode_materialises_the_schema_key_next_to_the_mcp_key() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");

    materialise_mcp(&path, Provider::OpenCode, &[]).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {},
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

/// The same definition, spelled the way OpenCode reads it: `stdio` →
/// `local`, one `command` array, `environment` for the env map.
#[test]
fn opencode_translates_a_stdio_definition_into_its_own_spelling() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    let servers = vec![
        McpServerDef::new("docs", "stdio")
            .command("npx")
            .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
            .env(std::collections::BTreeMap::from([(
                "TOKEN".to_owned(),
                "{env:TOKEN}".to_owned(),
            )])),
    ];

    materialise_mcp(&path, Provider::OpenCode, &servers).unwrap();

    let expected = json::to_canonical_string(&serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": {
            "docs": {
                "type": "local",
                "command": ["npx", "-y", "@acme/docs-mcp"],
                "environment": { "TOKEN": "{env:TOKEN}" },
            }
        }
    }))
    .unwrap();
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
}

#[test]
fn opencode_spells_a_remote_definition_with_type_remote_and_a_url() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    let servers = vec![McpServerDef::new("sentry", "sse").url("https://mcp.example.com/mcp")];

    materialise_mcp(&path, Provider::OpenCode, &servers).unwrap();

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(on_disk.contains("\"type\": \"remote\""), "was: {on_disk}");
    assert!(
        on_disk.contains("\"url\": \"https://mcp.example.com/mcp\""),
        "was: {on_disk}"
    );
}

/// `opencode.json` is OpenCode's *general* config. The user's other keys
/// must survive a sync that replaces the `mcp` key.
#[test]
fn an_existing_opencode_config_keeps_its_other_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    fs::write_text(
        &path,
        &json::to_canonical_string(&serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": { "stale": { "type": "local", "command": ["old"] } },
        }))
        .unwrap(),
    )
    .unwrap();

    let servers = vec![McpServerDef::new("docs", "stdio").command("npx")];
    assert_eq!(
        materialise_mcp(&path, Provider::OpenCode, &servers).unwrap(),
        Change::Updated
    );

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(
        on_disk.contains("claude-sonnet-4-5"),
        "the user's model must survive: {on_disk}"
    );
    assert!(
        !on_disk.contains("\"stale\""),
        "the mcp key must be replaced: {on_disk}"
    );
    assert!(
        on_disk.contains("\"docs\""),
        "the manifest's servers must land: {on_disk}"
    );

    // And the next sync touches nothing.
    assert_eq!(
        materialise_mcp(&path, Provider::OpenCode, &servers).unwrap(),
        Change::Unchanged
    );
}

// -- materialise_mcp: never clobber what cannot be parsed ----------------

#[test]
fn a_config_that_is_not_an_object_is_refused_not_clobbered() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    fs::write_text(&path, "[1, 2, 3]").unwrap();

    let error = materialise_mcp(&path, Provider::OpenCode, &[])
        .expect_err("an array cannot take an mcp key");

    let failure: Failure = error.into();
    assert_eq!(failure.code, "harness.mcp_not_an_object");
    assert_eq!(
        fs::read_text(&path).unwrap().unwrap(),
        "[1, 2, 3]",
        "the file must be left exactly as it was"
    );
}

#[test]
fn a_config_that_is_not_valid_json_is_refused_not_clobbered() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    fs::write_text(&path, "{ not json").unwrap();

    let error = materialise_mcp(&path, Provider::ClaudeCode, &[]).expect_err("unparseable");

    let failure: Failure = error.into();
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "harness.check_mcp_config"),
        "expected the mcp fix action, got {:?}",
        failure.fix_actions
    );
    assert_eq!(
        fs::read_text(&path).unwrap().unwrap(),
        "{ not json",
        "the file must be left exactly as it was"
    );
}

// -- remove_mcp ----------------------------------------------------------

#[test]
fn removing_an_exclusively_mcp_file_deletes_it() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join(".mcp.json");
    materialise_mcp(&path, Provider::ClaudeCode, &[]).unwrap();

    assert_eq!(
        remove_mcp(&path, Provider::ClaudeCode).unwrap(),
        Change::Removed
    );
    assert!(!fs::exists(&path).unwrap());
}

#[test]
fn removing_the_mcp_key_keeps_a_file_with_other_keys() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("opencode.json");
    fs::write_text(
        &path,
        &json::to_canonical_string(&serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": {},
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        remove_mcp(&path, Provider::OpenCode).unwrap(),
        Change::Removed
    );

    let on_disk = fs::read_text(&path).unwrap().unwrap();
    assert!(
        on_disk.contains("claude-sonnet-4-5"),
        "the user's keys must survive: {on_disk}"
    );
    assert!(
        !on_disk.contains("\"mcp\""),
        "the mcp key must be gone: {on_disk}"
    );
}

#[test]
fn removing_mcp_when_there_is_nothing_to_remove_is_unchanged() {
    let (_guard, dir) = utf8_temp_dir();

    let absent = dir.join(".mcp.json");
    assert_eq!(
        remove_mcp(&absent, Provider::ClaudeCode).unwrap(),
        Change::Unchanged
    );

    let without_mcp = dir.join("opencode.json");
    fs::write_text(&without_mcp, "{\"model\": \"x\"}").unwrap();
    assert_eq!(
        remove_mcp(&without_mcp, Provider::OpenCode).unwrap(),
        Change::Unchanged
    );
    assert_eq!(
        fs::read_text(&without_mcp).unwrap().unwrap(),
        "{\"model\": \"x\"}",
        "a file with no mcp key is not rewritten"
    );
}
