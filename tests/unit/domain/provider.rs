#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

// -- accessors, per variant: literals, not computed ---------------------

#[test]
fn claude_code_accessors() {
    let provider = Provider::ClaudeCode;
    assert_eq!(provider.id(), "claude-code");
    assert_eq!(provider.config_dir(), ".claude");
    assert_eq!(provider.instruction_file(), "CLAUDE.md");
    assert_eq!(provider.commands_dir(), ".claude/commands");
    assert_eq!(provider.skills_dir(), ".claude/skills");
    assert_eq!(provider.mcp_config_path(), ".mcp.json");
    assert_eq!(provider.mcp_key(), "mcpServers");
}

#[test]
fn opencode_accessors() {
    let provider = Provider::OpenCode;
    assert_eq!(provider.id(), "opencode");
    assert_eq!(provider.config_dir(), ".opencode");
    assert_eq!(provider.instruction_file(), "AGENTS.md");
    assert_eq!(provider.commands_dir(), ".opencode/commands");
    assert_eq!(provider.skills_dir(), ".opencode/skills");
    assert_eq!(provider.mcp_config_path(), "opencode.json");
    assert_eq!(provider.mcp_key(), "mcp");
}
#[test]
fn omp_accessors() {
    let provider = Provider::Omp;
    assert_eq!(provider.id(), "omp");
    assert_eq!(provider.config_dir(), ".omp");
    assert_eq!(provider.instruction_file(), "AGENTS.md");
    assert_eq!(provider.commands_dir(), ".omp/commands");
    assert_eq!(provider.skills_dir(), ".omp/skills");
    assert_eq!(provider.mcp_config_path(), ".omp/mcp.json");
    assert_eq!(provider.mcp_key(), "mcpServers");
}

// -- Display --------------------------------------------------------------

#[test]
fn display_is_the_id() {
    assert_eq!(Provider::ClaudeCode.to_string(), "claude-code");
    assert_eq!(Provider::OpenCode.to_string(), "opencode");
    assert_eq!(Provider::Omp.to_string(), "omp");
}

// -- serde: Serialize is a bare string, never the variant name -----------

#[test]
fn serialize_is_the_wire_string_not_the_variant_name() {
    assert_eq!(
        serde_json::to_string(&Provider::ClaudeCode).unwrap(),
        r#""claude-code""#
    );
    assert_eq!(
        serde_json::to_string(&Provider::OpenCode).unwrap(),
        r#""opencode""#
    );
    assert_eq!(serde_json::to_string(&Provider::Omp).unwrap(), r#""omp""#);
}

#[test]
fn deserializing_a_well_formed_id_round_trips() {
    let provider: Provider = serde_json::from_str(r#""claude-code""#).unwrap();
    assert_eq!(provider, Provider::ClaudeCode);

    let provider: Provider = serde_json::from_str(r#""opencode""#).unwrap();
    assert_eq!(provider, Provider::OpenCode);
    let provider: Provider = serde_json::from_str(r#""omp""#).unwrap();
    assert_eq!(provider, Provider::Omp);
}

/// A hand-edited `ivar.json` naming a harness that does not exist must fail
/// clearly, naming the valid options — not panic, and not silently pick a
/// default.
#[test]
fn deserializing_an_unknown_id_fails_naming_the_valid_options() {
    let result: Result<Provider, _> = serde_json::from_str(r#""claude""#);
    let error = result.expect_err("unknown id must be rejected");
    let message = error.to_string();
    assert!(message.contains("claude-code"));
    assert!(message.contains("opencode"));
    assert!(message.contains("omp"));
}

// -- FromStr agrees with Deserialize --------------------------------------

#[test]
fn from_str_accepts_every_id() {
    assert_eq!(
        "claude-code".parse::<Provider>().unwrap(),
        Provider::ClaudeCode
    );
    assert_eq!("opencode".parse::<Provider>().unwrap(), Provider::OpenCode);
    assert_eq!("omp".parse::<Provider>().unwrap(), Provider::Omp);
}

#[test]
fn from_str_and_deserialize_agree_on_an_unknown_id() {
    let from_str_error = "claude".parse::<Provider>().unwrap_err();
    let deserialize_error = serde_json::from_str::<Provider>(r#""claude""#).unwrap_err();
    assert_eq!(from_str_error.to_string(), deserialize_error.to_string());
}

#[test]
fn from_str_error_names_the_rejected_value() {
    let error = "not-a-provider".parse::<Provider>().unwrap_err();
    assert_eq!(
        error,
        InvalidProvider::UnknownId {
            given: "not-a-provider".to_owned()
        }
    );
}

// -- InvalidProvider -> Failure -------------------------------------------

#[test]
fn invalid_provider_converts_to_a_blocked_failure_with_a_fix_naming_the_options() {
    let failure: Failure = InvalidProvider::UnknownId {
        given: "claude".to_owned(),
    }
    .into();
    assert_eq!(failure.code, "provider.unknown_id");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);
    assert!(failure.fix_actions[0].what.contains("claude-code"));
    assert!(failure.fix_actions[0].what.contains("opencode"));
    assert!(failure.fix_actions[0].what.contains("omp"));
}

// -- ALL is exhaustive -----------------------------------------------------

/// The cheap way to make forgetting a new variant in `ALL` a compile
/// error: this `match` has no wildcard arm, so adding a variant to
/// `Provider` without adding it here fails to compile, forcing the two to
/// stay in lockstep.
#[test]
fn all_is_exhaustive() {
    fn covers_every_variant(provider: Provider) {
        match provider {
            Provider::ClaudeCode | Provider::OpenCode | Provider::Omp => {}
        }
    }

    for provider in Provider::ALL {
        covers_every_variant(provider);
    }
    assert_eq!(Provider::ALL.len(), 3);
}
