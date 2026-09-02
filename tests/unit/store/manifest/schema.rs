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

/// Follow a `$ref` from the root schema, returning the referenced definition.
fn follow_ref<'a>(root: &'a Value, val: &'a Value) -> &'a Value {
    match val.get("$ref").and_then(Value::as_str) {
        Some(ref_path) if ref_path.starts_with("#/") => root
            .pointer(&ref_path[1..])
            .unwrap_or_else(|| panic!("unresolved $ref: {ref_path}")),
        _ => val,
    }
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

// -- closed objects -------------------------------------------------------

#[test]
fn root_object_is_closed() {
    let s = schema();
    assert_eq!(
        s.get("additionalProperties"),
        Some(&json!(false)),
        "root must be closed"
    );
}

#[test]
fn all_object_types_in_defs_are_closed() {
    let s = schema();
    let defs = s
        .get("$defs")
        .expect("schema must have $defs from schemars derivation");
    for (name, def) in defs.as_object().expect("$defs must be an object") {
        if def.get("type").and_then(Value::as_str) == Some("object") {
            assert_eq!(
                def.get("additionalProperties"),
                Some(&json!(false)),
                "$defs.{name} must be a closed object (additionalProperties: false)"
            );
        }
    }
}

// -- provider and integration enums --------------------------------------

#[test]
fn provider_enum_values() {
    let s = schema();
    let defs = s.get("$defs").expect("must have $defs");
    let provider = defs.get("Provider").expect("must have Provider in $defs");

    let values: Vec<&str> = provider
        .get("enum")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    assert!(values.contains(&"claude-code"), "must include claude-code");
    assert!(values.contains(&"opencode"), "must include opencode");
}

#[test]
fn integration_via_enum_values() {
    let s = schema();
    let defs = s.get("$defs").expect("must have $defs");
    let via = defs
        .get("IntegrationVia")
        .expect("must have IntegrationVia in $defs");

    let variants: Vec<&str> = via
        .get("oneOf")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("const").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();

    assert!(variants.contains(&"pr"), "must include pr");
    assert!(variants.contains(&"local"), "must include local");
}

#[test]
fn integration_strategy_enum_values() {
    let s = schema();
    let defs = s.get("$defs").expect("must have $defs");
    let strategy = defs
        .get("IntegrationStrategy")
        .expect("must have IntegrationStrategy in $defs");

    let variants: Vec<&str> = strategy
        .get("oneOf")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("const").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();

    assert!(variants.contains(&"squash"), "must include squash");
    assert!(variants.contains(&"merge"), "must include merge");
    assert!(variants.contains(&"rebase"), "must include rebase");
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
    let defs = s.get("$defs").expect("must have $defs");
    let repo = defs.get("Repo").expect("must have Repo in $defs");
    let default_branch = repo.get("properties").unwrap().get("default_branch");
    assert!(
        default_branch.is_some(),
        "repo items must have default_branch"
    );
    let resolved = follow_ref(&s, default_branch.unwrap());
    assert_eq!(
        resolved.get("type").and_then(Value::as_str),
        Some("string"),
        "default_branch must be a string"
    );
}

// -- repo checks ----------------------------------------------------------

#[test]
fn repo_has_checks_array() {
    let s = schema();
    let defs = s.get("$defs").expect("must have $defs");
    let repo = defs.get("Repo").expect("must have Repo in $defs");
    let checks = repo.get("properties").unwrap().get("checks");
    assert!(checks.is_some(), "repo items must have checks");
    let resolved = follow_ref(&s, checks.unwrap());
    assert_eq!(
        resolved.get("type").and_then(Value::as_str),
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
            let resolved = follow_ref(&s, oauth);
            let oauth_props = resolved.get("properties").unwrap();
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
fn type_definitions_have_descriptions() {
    let s = schema();
    let defs = s.get("$defs").expect("must have $defs");
    for name in [
        "Providers",
        "Repo",
        "IntegrationPolicy",
        "Skills",
        "Targets",
        "McpServerDef",
        "McpOauth",
    ] {
        let def = defs
            .get(name)
            .unwrap_or_else(|| panic!("$defs must contain {name}"));
        assert!(
            def.get("description").and_then(Value::as_str).is_some(),
            "$defs.{name} must have a description"
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
fn mcp_http_branch_requires_url() {
    let s = schema();
    let http_branch = find_mcp_branch(&s, "http");

    let required: Vec<&str> = http_branch
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(required.contains(&"name"), "http branch must require name");
    assert!(required.contains(&"type"), "http branch must require type");
    assert!(required.contains(&"url"), "http branch must require url");
}

#[test]
fn mcp_http_branch_forbids_command_args_env_by_absence() {
    let s = schema();
    let http_branch = find_mcp_branch(&s, "http");

    // The http branch must not define command, args, or env as properties,
    // and must be a closed object (additionalProperties: false).
    let props = http_branch
        .get("properties")
        .expect("http branch must have properties");
    assert!(
        props.get("command").is_none(),
        "http branch must not have command in properties"
    );
    assert!(
        props.get("args").is_none(),
        "http branch must not have args in properties"
    );
    assert!(
        props.get("env").is_none(),
        "http branch must not have env in properties"
    );
    assert_eq!(
        http_branch.get("additionalProperties"),
        Some(&json!(false)),
        "http branch must be closed"
    );
}

#[test]
fn mcp_local_branch_requires_command() {
    let s = schema();
    let local_branch = find_mcp_branch(&s, "local");

    let required: Vec<&str> = local_branch
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(required.contains(&"name"), "local branch must require name");
    assert!(required.contains(&"type"), "local branch must require type");
    assert!(
        required.contains(&"command"),
        "local branch must require command"
    );
}

#[test]
fn mcp_local_branch_forbids_url_by_absence() {
    let s = schema();
    let local_branch = find_mcp_branch(&s, "local");

    // The local branch must not define url as a property,
    // and must be a closed object (additionalProperties: false).
    let props = local_branch
        .get("properties")
        .expect("local branch must have properties");
    assert!(
        props.get("url").is_none(),
        "local branch must not have url in properties"
    );
    assert_eq!(
        local_branch.get("additionalProperties"),
        Some(&json!(false)),
        "local branch must be closed"
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

// -- Version constraint ---------------------------------------------------

#[test]
fn version_is_const_current_version() {
    let s = schema();
    let version = s.get("properties").unwrap().get("version").unwrap();
    assert_eq!(
        version.get("const").and_then(Value::as_i64),
        Some(super::super::model::CURRENT_VERSION as i64),
        "version must be const CURRENT_VERSION"
    );
}

// -- Type-schema binding (schemars derivation) ----------------------------

/// Proves the binding between Rust types and the schema.
/// If a field is added to Manifest or any reachable type, schemars will
/// include it in the schema_for!(Manifest) output, which changes generate(),
/// which changes the artifact. The drift gate test will fail if the artifact
/// isn't regenerated.
#[test]
fn schema_defs_cover_manifest_reachable_types() {
    let s = schema();
    let defs = s
        .get("$defs")
        .expect("schema must have $defs from schemars derivation");
    for type_name in [
        "Providers",
        "Provider",
        "Repo",
        "IntegrationPolicy",
        "IntegrationVia",
        "IntegrationStrategy",
        "Skills",
        "Targets",
        "McpServerDef",
        "McpOauth",
    ] {
        assert!(
            defs.get(type_name).is_some(),
            "$defs must contain {type_name} — if this fails, the type was \
             removed from the reachable set or schemars changed its $defs key"
        );
    }
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
