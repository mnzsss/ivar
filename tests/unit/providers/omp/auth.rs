// tests/unit/providers/omp/auth.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::infra::fs::{TempDir, write_sensitive_atomic};
use crate::infra::oauth::Tokens;
use crate::infra::proc::Output;
use crate::providers::omp::auth::{
    credential_binding_from, credential_json, import_command, logout_command, parse_import_result,
    verify_result_from_output,
};

#[test]
fn binding_uses_active_profile_or_falls_back() {
    let url = "https://acme.example.com/mcp?tenant=123";

    // 1. OMP_PROFILE=work
    let binding_omp = credential_binding_from(Some("work"), None, url);
    assert_eq!(
        binding_omp,
        "mcp_oauth:profile:work:https://acme.example.com/mcp?tenant=123"
    );

    // 2. PI_PROFILE=legacy (OMP_PROFILE unset)
    let binding_pi = credential_binding_from(None, Some("legacy"), url);
    assert_eq!(
        binding_pi,
        "mcp_oauth:profile:legacy:https://acme.example.com/mcp?tenant=123"
    );

    // 3. Neither set -> "default"
    let binding_default = credential_binding_from(None, None, url);
    assert_eq!(
        binding_default,
        "mcp_oauth:profile:default:https://acme.example.com/mcp?tenant=123"
    );

    // 4. OMP_PROFILE takes precedence over PI_PROFILE
    let binding_both = credential_binding_from(Some("work"), Some("legacy"), url);
    assert_eq!(
        binding_both,
        "mcp_oauth:profile:work:https://acme.example.com/mcp?tenant=123"
    );
}

#[test]
fn binding_preserves_url_verbatim() {
    let raw_urls = [
        "https://acme.example.com/mcp?tenant=123&foo=BAR%20BAZ",
        "https://ACME.EXAMPLE.COM/mcp/",
        "http://127.0.0.1:8080/path/to/server?q=1#frag",
    ];

    for url in raw_urls {
        let binding = credential_binding_from(Some("test"), None, url);
        assert_eq!(binding, format!("mcp_oauth:profile:test:{url}"));
    }
}

#[test]
fn argv_shape_matches_omp_auth_broker_cli() {
    let binding = "mcp_oauth:profile:work:https://acme.example.com/mcp";
    let path = camino::Utf8Path::new("/tmp/cred-123.json");

    let import_cmd = import_command(path, binding);
    assert_eq!(
        import_cmd.display(),
        "omp auth-broker import /tmp/cred-123.json --provider mcp_oauth:profile:work:https://acme.example.com/mcp --json"
    );
    assert!(!import_cmd.display().contains("--file"));

    let logout_cmd = logout_command(binding);
    assert_eq!(
        logout_cmd.display(),
        "omp auth-broker logout --provider mcp_oauth:profile:work:https://acme.example.com/mcp --json"
    );
}

#[test]
fn ordering_logout_built_and_run_before_import() {
    let binding = "mcp_oauth:profile:work:https://acme.example.com/mcp";
    let path = camino::Utf8Path::new("/tmp/cred.json");

    let logout = logout_command(binding);
    let import = import_command(path, binding);

    let sequence = [logout.display(), import.display()];
    assert_eq!(
        sequence,
        [
            "omp auth-broker logout --provider mcp_oauth:profile:work:https://acme.example.com/mcp --json".to_owned(),
            "omp auth-broker import /tmp/cred.json --provider mcp_oauth:profile:work:https://acme.example.com/mcp --json".to_owned()
        ]
    );
}

#[test]
fn temp_file_written_with_0600_and_cleaned_up() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let cred_path = temp_dir.path().join("credential.json");

    let content = b"{\"type\":\"oauth\",\"access_token\":\"at\",\"refresh_token\":\"rt\",\"expired\":\"2027-01-01T00:00:00Z\"}";
    write_sensitive_atomic(&cred_path, content).unwrap();

    assert!(cred_path.exists());
    #[cfg(unix)]
    {
        let metadata = std::fs::metadata(cred_path.as_std_path()).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    let raw = std::fs::read_to_string(cred_path.as_std_path()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(json.get("type").and_then(|v| v.as_str()), Some("oauth"));
    assert_eq!(
        json.get("access_token").and_then(|v| v.as_str()),
        Some("at")
    );
    assert_eq!(
        json.get("refresh_token").and_then(|v| v.as_str()),
        Some("rt")
    );
    assert_eq!(
        json.get("expired").and_then(|v| v.as_str()),
        Some("2027-01-01T00:00:00Z")
    );

    let dir_path = temp_dir.path().to_owned();
    drop(temp_dir);
    assert!(!dir_path.exists());
    assert!(!cred_path.exists());
}

#[test]
fn rfc3339_expiry_conversion() {
    // 1800000000 seconds = 2027-01-15T08:00:00Z
    let tokens = Tokens {
        access_token: "secret_at".to_owned(),
        refresh_token: Some("secret_rt".to_owned()),
        expires_at: Some(1800000000.0),
        scope: None,
    };

    let json_str = credential_json(&tokens).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.get("type").and_then(|v| v.as_str()), Some("oauth"));
    assert_eq!(
        parsed.get("access_token").and_then(|v| v.as_str()),
        Some("secret_at")
    );
    assert_eq!(
        parsed.get("refresh_token").and_then(|v| v.as_str()),
        Some("secret_rt")
    );
    assert_eq!(
        parsed.get("expired").and_then(|v| v.as_str()),
        Some("2027-01-15T08:00:00.000000000Z")
    );

    // Must refuse tokens lacking refresh_token
    let tokens_no_refresh = Tokens {
        access_token: "secret_at".to_owned(),
        refresh_token: None,
        expires_at: Some(1800000000.0),
        scope: None,
    };
    assert!(credential_json(&tokens_no_refresh).is_err());

    // Must refuse tokens lacking expires_at
    let tokens_no_expiry = Tokens {
        access_token: "secret_at".to_owned(),
        refresh_token: Some("secret_rt".to_owned()),
        expires_at: None,
        scope: None,
    };
    assert!(credential_json(&tokens_no_expiry).is_err());
}

#[test]
fn structured_validation_requires_binding_in_imported_array() {
    let binding = "mcp_oauth:profile:default:https://acme.example.com/mcp";

    // 1. Success case: present in imported[]
    let success_json = format!(
        r#"{{"dryRun":false,"imported":[{{"provider":"{binding}","path":"/tmp/cred.json"}}],"skipped":[]}}"#
    );
    assert!(parse_import_result(&success_json, binding).is_ok());

    // 2. Failure case: absent from imported[]
    let empty_imported_json = r#"{"dryRun":false,"imported":[],"skipped":[]}"#;
    let err_empty = parse_import_result(empty_imported_json, binding).unwrap_err();
    assert_eq!(err_empty.code, "omp_auth.import_failed");

    // 3. Failure case: present in skipped[]
    let skipped_json = format!(
        r#"{{"dryRun":false,"imported":[],"skipped":[{{"provider":"{binding}","reason":"already_exists"}}]}}"#
    );
    let err_skipped = parse_import_result(&skipped_json, binding).unwrap_err();
    assert_eq!(err_skipped.code, "omp_auth.import_failed");

    // 4. Failure case: unparseable JSON
    let invalid_json = "not valid json";
    let err_invalid = parse_import_result(invalid_json, binding).unwrap_err();
    assert_eq!(err_invalid.code, "omp_auth.invalid_json");
}

#[test]
fn redaction_no_secrets_in_errors_or_debug() {
    let tokens = Tokens {
        access_token: "super_secret_access_token_12345".to_owned(),
        refresh_token: Some("super_secret_refresh_token_67890".to_owned()),
        expires_at: None,
        scope: None,
    };

    let err = credential_json(&tokens).unwrap_err();
    let err_str = format!("{err:?} {err}");
    assert!(!err_str.contains("super_secret_access_token_12345"));
    assert!(!err_str.contains("super_secret_refresh_token_67890"));

    let binding = "mcp_oauth:profile:default:https://acme.example.com/mcp";
    let path = camino::Utf8Path::new("/tmp/cred.json");
    let cmd = import_command(path, binding);
    let cmd_str = format!("{cmd:?} {}", cmd.display());
    assert!(!cmd_str.contains("super_secret_access_token_12345"));
    assert!(!cmd_str.contains("super_secret_refresh_token_67890"));
}

#[test]
fn token_value_in_stdout_does_not_leak_into_verify_failure() {
    let secret_token = "secret_access_token_should_never_leak_99999";
    let binding = "mcp_oauth:profile:default:https://acme.example.com/mcp";

    // When `omp token` exits non-zero, but has written the token to stdout and stderr is empty:
    let output = Output {
        code: Some(1),
        stdout: secret_token.to_owned(),
        stderr: String::new(),
    };

    let err = verify_result_from_output(&output, binding).unwrap_err();
    let rendered_debug = format!("{err:?}");
    let rendered_display = format!("{err}");
    let rendered_what = &err.what;
    let rendered_expected = err.expected.as_deref().unwrap_or_default();
    let rendered_actual = err.actual.as_deref().unwrap_or_default();

    assert!(
        !rendered_debug.contains(secret_token),
        "token leaked into Debug: {rendered_debug}"
    );
    assert!(
        !rendered_display.contains(secret_token),
        "token leaked into Display: {rendered_display}"
    );
    assert!(
        !rendered_what.contains(secret_token),
        "token leaked into what: {rendered_what}"
    );
    assert!(
        !rendered_expected.contains(secret_token),
        "token leaked into expected: {rendered_expected}"
    );
    assert!(
        !rendered_actual.contains(secret_token),
        "token leaked into actual: {rendered_actual}"
    );
}
