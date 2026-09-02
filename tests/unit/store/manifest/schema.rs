//! Unit tests for `crate::store::manifest::schema`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::{Value, json};

use super::generate;

fn schema() -> Value {
    generate()
}

/// Navigate to the MCP `oneOf` array inside `properties.mcp.items`.
fn mcp_one_of(s: &Value) -> &[Value] {
    s.get("properties")
        .expect("schema must have properties")
        .get("mcp")
        .expect("mcp property must exist")
        .get("items")
        .expect("mcp must be an array with items")
        .get("oneOf")
        .expect("mcp items must use oneOf")
        .as_array()
        .expect("oneOf must be an array")
}

/// Find the branch in `mcp_one_of` whose `type.const` equals `transport`.
fn find_mcp_branch<'a>(s: &'a Value, transport: &str) -> &'a Value {
    mcp_one_of(s)
        .iter()
        .find(|b| {
            b.get("properties")
                .and_then(|p| p.get("type"))
                .and_then(|t| t.get("const"))
                .and_then(|c| c.as_str())
                == Some(transport)
        })
        .unwrap_or_else(|| panic!("must have a `{transport}` branch"))
}

// -- draft and metadata ---------------------------------------------------

#[test]
fn schema_is_draft_2020_12() {
    let s = schema();
    assert_eq!(
        s.get("$schema").and_then(Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema"),
        "must declare JSON Schema draft 2020-12"
    );
}

#[test]
fn schema_has_id_and_title() {
    let s = schema();
    assert_eq!(
        s.get("$id").and_then(Value::as_str),
        Some("https://ivar.run/ivar.schema.json"),
        "$id must match the canonical URL"
    );
    assert_eq!(
        s.get("title").and_then(Value::as_str),
        Some("ivar.json"),
        "title must be the file name"
    );
}

// -- top-level properties -------------------------------------------------

#[test]
fn schema_describes_all_top_level_properties() {
    let s = schema();
    let props = s.get("properties").expect("must have properties");
    for key in [
        "$schema",
        "version",
        "name",
        "providers",
        "repos",
        "integration",
        "skills",
        "mcp",
    ] {
        assert!(
            props.get(key).is_some(),
            "top-level property `{key}` must be described"
        );
    }
}

#[test]
fn required_top_level_fields_are_declared() {
    let s = schema();
    let required = s
        .get("required")
        .and_then(Value::as_array)
        .expect("must have required array");
    let required: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    for key in ["version", "name", "providers", "repos", "integration"] {
        assert!(
            required.contains(&key),
            "`{key}` must be in the top-level required list, got: {required:?}"
        );
    }
}

#[test]
fn skills_and_mcp_are_optional() {
    let s = schema();
    let required = s
        .get("required")
        .and_then(Value::as_array)
        .expect("must have required array");
    let required: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    assert!(!required.contains(&"skills"), "skills must not be required");
    assert!(!required.contains(&"mcp"), "mcp must not be required");
}

#[test]
fn objects_are_closed() {
    let s = schema();
    assert_eq!(
        s.get("additionalProperties"),
        Some(&json!(false)),
        "root must be closed"
    );

    // Check nested objects too
    let props = s.get("properties").unwrap();
    for key in ["providers", "repos", "integration", "skills"] {
        if let Some(obj) = props.get(key)
            && obj.get("type").and_then(Value::as_str) == Some("object")
        {
            assert_eq!(
                obj.get("additionalProperties"),
                Some(&json!(false)),
                "`{key}` object must be closed"
            );
        }
    }
}

// -- provider and integration enums --------------------------------------

#[test]
fn provider_enum_values() {
    let s = schema();
    let props = s.get("properties").unwrap();
    let providers = props.get("providers").unwrap();
    let available = providers
        .get("properties")
        .unwrap()
        .get("available")
        .unwrap();
    let items = available.get("items").unwrap();

    // available items should be an enum with the two provider ids
    let values: Vec<&str> = items
        .get("enum")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if values.is_empty() {
        // Try the enum approach: check if it's a oneOf of const values
        let one_of = items.get("oneOf").or_else(|| items.get("anyOf"));
        if let Some(variants) = one_of.and_then(Value::as_array) {
            let consts: Vec<&str> = variants
                .iter()
                .filter_map(|v| v.get("const").and_then(Value::as_str))
                .collect();
            assert!(consts.contains(&"claude-code"), "must include claude-code");
            assert!(consts.contains(&"opencode"), "must include opencode");
        } else {
            panic!("provider enum must use enum, oneOf, or anyOf with const values");
        }
    } else {
        assert!(values.contains(&"claude-code"), "must include claude-code");
        assert!(values.contains(&"opencode"), "must include opencode");
    }
}

#[test]
fn integration_via_enum_values() {
    let s = schema();
    let props = s.get("properties").unwrap();
    let integration = props.get("integration").unwrap();
    let via = integration.get("properties").unwrap().get("via").unwrap();

    let values: Vec<&str> = via
        .get("enum")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if values.is_empty() {
        let one_of = via.get("oneOf").or_else(|| via.get("anyOf"));
        if let Some(variants) = one_of.and_then(Value::as_array) {
            let consts: Vec<&str> = variants
                .iter()
                .filter_map(|v| v.get("const").and_then(Value::as_str))
                .collect();
            assert!(consts.contains(&"pr"), "must include pr");
            assert!(consts.contains(&"local"), "must include local");
        } else {
            panic!("integrationVia must use enum, oneOf, or anyOf with const values");
        }
    } else {
        assert!(values.contains(&"pr"), "must include pr");
        assert!(values.contains(&"local"), "must include local");
    }
}

#[test]
fn integration_strategy_enum_values() {
    let s = schema();
    let props = s.get("properties").unwrap();
    let integration = props.get("integration").unwrap();
    let strategy = integration
        .get("properties")
        .unwrap()
        .get("strategy")
        .unwrap();

    let values: Vec<&str> = strategy
        .get("enum")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if values.is_empty() {
        let one_of = strategy.get("oneOf").or_else(|| strategy.get("anyOf"));
        if let Some(variants) = one_of.and_then(Value::as_array) {
            let consts: Vec<&str> = variants
                .iter()
                .filter_map(|v| v.get("const").and_then(Value::as_str))
                .collect();
            assert!(consts.contains(&"squash"), "must include squash");
            assert!(consts.contains(&"merge"), "must include merge");
            assert!(consts.contains(&"rebase"), "must include rebase");
        } else {
            panic!("integrationStrategy must use enum, oneOf, or anyOf with const values");
        }
    } else {
        assert!(values.contains(&"squash"), "must include squash");
        assert!(values.contains(&"merge"), "must include merge");
        assert!(values.contains(&"rebase"), "must include rebase");
    }
}

// -- name and branch constraints ------------------------------------------

#[test]
fn hall_name_is_a_string() {
    let s = schema();
    let props = s.get("properties").unwrap();
    let name = props.get("name").unwrap();
    assert_eq!(
        name.get("type").and_then(Value::as_str),
        Some("string"),
        "name must be a string"
    );
}

#[test]
fn repo_default_branch_is_a_string() {
    let s = schema();
    let props = s.get("properties").unwrap();
    let repos = props.get("repos").unwrap();
    let items = repos.get("items").unwrap();
    let default_branch = items.get("properties").unwrap().get("default_branch");
    assert!(
        default_branch.is_some(),
        "repo items must have default_branch"
    );
    assert_eq!(
        default_branch.unwrap().get("type").and_then(Value::as_str),
        Some("string"),
        "default_branch must be a string"
    );
}

// -- repo checks ----------------------------------------------------------

#[test]
fn repo_has_checks_array() {
    let s = schema();
    let props = s.get("properties").unwrap();
    let repos = props.get("repos").unwrap();
    let items = repos.get("items").unwrap();
    let checks = items.get("properties").unwrap().get("checks");
    assert!(checks.is_some(), "repo items must have checks");
    let checks = checks.unwrap();
    assert_eq!(
        checks.get("type").and_then(Value::as_str),
        Some("array"),
        "checks must be an array"
    );
}

// -- OAuth reference fields -----------------------------------------------

#[test]
fn mcp_oauth_has_reference_fields() {
    let s = schema();

    for branch in mcp_one_of(&s) {
        if let Some(props) = branch.get("properties")
            && let Some(oauth) = props.get("oauth")
        {
            let oauth_props = oauth.get("properties").unwrap();
            assert!(
                oauth_props.get("client_id").is_some(),
                "oauth must have client_id"
            );
            assert!(
                oauth_props.get("client_secret_env").is_some(),
                "oauth must have client_secret_env"
            );
        }
    }
}

// -- descriptions and examples --------------------------------------------

#[test]
fn schema_has_description() {
    let s = schema();
    assert!(
        s.get("description").and_then(Value::as_str).is_some(),
        "root schema must have a description"
    );
}

#[test]
fn schema_properties_have_descriptions() {
    let s = schema();
    let props = s.get("properties").unwrap();
    for key in ["version", "name", "providers", "repos", "integration"] {
        let prop = props.get(key).unwrap();
        assert!(
            prop.get("description").and_then(Value::as_str).is_some(),
            "property `{key}` must have a description"
        );
    }
}

// -- MCP oneOf: http and local branches -----------------------------------

#[test]
fn mcp_is_one_of_two_branches() {
    let s = schema();
    let branches = mcp_one_of(&s);
    assert_eq!(branches.len(), 2, "mcp oneOf must have exactly 2 branches");
}

#[test]
fn mcp_http_branch_requires_url_and_forbids_command_args_env() {
    let s = schema();
    let http_branch = find_mcp_branch(&s, "http");

    // Required fields
    let required: Vec<&str> = http_branch
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(required.contains(&"url"), "http branch must require url");

    // Forbidden fields: command, args, env
    let not = http_branch.get("not").expect("http branch must have not");
    let not_required: Vec<&str> = not
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        not_required.contains(&"command"),
        "http branch must forbid command"
    );
    assert!(
        not_required.contains(&"args"),
        "http branch must forbid args"
    );
    assert!(not_required.contains(&"env"), "http branch must forbid env");
}

#[test]
fn mcp_local_branch_requires_command_and_forbids_url() {
    let s = schema();
    let local_branch = find_mcp_branch(&s, "local");

    // Required fields
    let required: Vec<&str> = local_branch
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        required.contains(&"command"),
        "local branch must require command"
    );

    // Forbidden fields: url
    let not = local_branch.get("not").expect("local branch must have not");
    let not_required: Vec<&str> = not
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        not_required.contains(&"url"),
        "local branch must forbid url"
    );
}

// -- MCP branches are closed objects --------------------------------------

#[test]
fn mcp_branches_are_closed_objects() {
    let s = schema();
    for branch in mcp_one_of(&s) {
        assert_eq!(
            branch.get("additionalProperties"),
            Some(&json!(false)),
            "MCP branch must be a closed object"
        );
    }
}

// -- Structural validation of representative values -----------------------

#[test]
fn accepted_http_mcp_value_structurally() {
    let s = schema();
    let http_branch = find_mcp_branch(&s, "http");

    // Verify the http branch has the right shape for an accepted value
    let props = http_branch.get("properties").unwrap();
    assert!(props.get("name").is_some(), "http must have name");
    assert!(props.get("type").is_some(), "http must have type");
    assert!(props.get("url").is_some(), "http must have url");
    assert!(props.get("oauth").is_some(), "http must have oauth");
}

#[test]
fn accepted_local_mcp_value_structurally() {
    let s = schema();
    let local_branch = find_mcp_branch(&s, "local");

    let props = local_branch.get("properties").unwrap();
    assert!(props.get("name").is_some(), "local must have name");
    assert!(props.get("type").is_some(), "local must have type");
    assert!(props.get("command").is_some(), "local must have command");
    assert!(props.get("args").is_some(), "local must have args");
    assert!(props.get("oauth").is_some(), "local must have oauth");
}

#[test]
fn rejected_stdio_is_not_in_mcp_one_of() {
    // The schema should not have a stdio branch
    let s = schema();
    let has_stdio = mcp_one_of(&s).iter().any(|b| {
        b.get("properties")
            .and_then(|p| p.get("type"))
            .and_then(|t| t.get("const"))
            .and_then(|c| c.as_str())
            == Some("stdio")
    });
    assert!(!has_stdio, "stdio must not appear as a valid MCP transport");
}

#[test]
fn rejected_sse_is_not_in_mcp_one_of() {
    let s = schema();
    let has_sse = mcp_one_of(&s).iter().any(|b| {
        b.get("properties")
            .and_then(|p| p.get("type"))
            .and_then(|t| t.get("const"))
            .and_then(|c| c.as_str())
            == Some("sse")
    });
    assert!(!has_sse, "sse must not appear as a valid MCP transport");
}

// -- Task 5: drift gate and package inclusion ------------------------------

fn schema_artifact_path() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("ivar.schema.json")
}

#[test]
fn schema_artifact_matches_model() {
    let mut generated = serde_json::to_string_pretty(&generate())
        .expect("generate() must produce serializable JSON");
    generated.push('\n');
    let path = schema_artifact_path();
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert_eq!(
        generated, expected,
        "generated schema must exactly match checked-in ivar.schema.json \
         (run `cargo run --example generate-manifest-schema` to update)"
    );
}

#[test]
fn schema_artifact_is_package_included() {
    let output = std::process::Command::new("cargo")
        .args(["package", "--allow-dirty", "--list"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo package --list must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ivar.schema.json"),
        "ivar.schema.json must be included in cargo package output, got:\n{stdout}"
    );
}
