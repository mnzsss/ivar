#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

// -- needs_preregistration: the allowlist table --------------------------

#[test]
fn figma_host_needs_preregistration() {
    assert!(needs_preregistration("mcp.figma.com"));
}

#[test]
fn other_hosts_do_not() {
    assert!(!needs_preregistration("mcp.linear.app"));
    assert!(!needs_preregistration("api.github.com"));
    assert!(!needs_preregistration(""));
}

// -- registration_failure: the 403-vs-generic split ----------------------

#[test]
fn a_403_names_the_allowlist_not_the_caller() {
    let failure = registration_failure(403, "forbidden".to_owned());
    assert_eq!(failure.code, "figma.register_client_http");
    let fix = failure.fix_actions.first().expect("a fix action");
    assert!(
        fix.what.contains("allowlist"),
        "fix should name the allowlist: {}",
        fix.what
    );
}

#[test]
fn other_statuses_get_a_generic_failure_with_no_fix() {
    let failure = registration_failure(500, "server error".to_owned());
    assert_eq!(failure.code, "figma.register_client_http");
    assert!(failure.fix_actions.is_empty());
    assert_eq!(failure.actual.as_deref(), Some("server error"));
}

// -- real 403, over the network --------------------------------------------
//
// Everything above tests pure helpers and cannot catch a regression in which
// *branch the transport takes* — ureq 3.x defaults to turning any non-2xx
// status into a transport error before application code sees it, which
// would make the 403 branch in `register_client_as` dead code even though
// `registration_failure` itself is correct. Only a real request exercises
// that. `client_name: "opencode"` is confirmed to 403 against Figma's
// registration endpoint (it's the name OpenCode's own dynamic registration
// sends, and it is not on the allowlist), so this hits the network for
// real and is `#[ignore]`d to keep CI offline. Run with
// `cargo test -- --ignored`.
#[test]
#[ignore = "hits the real Figma registration endpoint over the network"]
fn a_real_403_names_the_allowlist() {
    let error = register_client_as("http://127.0.0.1:19876/callback", "opencode").unwrap_err();
    assert_eq!(error.code, "figma.register_client_http");
    let fix = error.fix_actions.first().expect("a fix action on a 403");
    assert!(
        fix.what.contains("allowlist"),
        "fix should name the allowlist: {}",
        fix.what
    );
}

// -- discovery correctness (offline) ---------------------------------------

#[test]
fn parse_www_authenticate_resource_metadata_in_various_orders() {
    let header1 = r#"Bearer realm="Figma", resource_metadata="https://mcp.figma.com/.well-known/oauth-protected-resource""#;
    let header2 = r#"Bearer resource_metadata="https://mcp.figma.com/.well-known/oauth-protected-resource", realm="Figma""#;
    let expected = Some("https://mcp.figma.com/.well-known/oauth-protected-resource".to_owned());
    assert_eq!(parse_www_authenticate_resource_metadata(header1), expected);
    assert_eq!(parse_www_authenticate_resource_metadata(header2), expected);
}

#[test]
fn parse_www_authenticate_missing_or_malformed() {
    assert_eq!(
        parse_www_authenticate_resource_metadata("Bearer realm=Figma"),
        None
    );
    assert_eq!(
        parse_www_authenticate_resource_metadata("Bearer resource_metadata="),
        Some("".to_owned())
    );
}

#[test]
fn parse_resource_metadata_errors() {
    // No authorization_servers
    let malformed = r#"{"resource":"https://api.figma.com"}"#;
    assert!(parse_resource_metadata(malformed).is_err());
}

#[test]
fn build_well_known_url_roots_and_paths() {
    assert_eq!(
        build_well_known_url("https://auth.example.com").unwrap(),
        "https://auth.example.com/.well-known/oauth-authorization-server"
    );
    assert_eq!(
        build_well_known_url("https://auth.example.com/issuer/v1").unwrap(),
        "https://auth.example.com/issuer/v1/.well-known/oauth-authorization-server"
    );
}

#[test]
fn parse_authorization_metadata_errors() {
    // Missing token_endpoint
    let malformed = r#"{"authorization_endpoint":"https://auth.example.com/authorize"}"#;
    assert!(parse_authorization_metadata(malformed).is_err());
}

// -- reproduction of 405 error ---------------------------------------------
#[test]
fn discover_oauth_endpoints_repro_405() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{}", port);

    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();

            if request_line.starts_with("GET") {
                stream
                    .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                    .unwrap();
            } else if request_line.starts_with("POST") {
                stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer resource_metadata=\"http://localhost/metadata\"\r\n\r\n").unwrap();
            }
        }
    });

    // This should now succeed (or at least get past 405)
    let result = discover_oauth_endpoints(&url);
    // Now we assert success in proceeding past step 1!
    // But Step 2 will fail because http://localhost/metadata doesn't exist.
    // That's fine, we just want to prove we triggered the 401 and parsed the header correctly.
    assert!(
        result.is_err(),
        "Expected error on Step 2 (not found), but got: {:?}",
        result
    );
    let err = result.err().unwrap();
    assert!(
        err.code.contains("figma.discover_resource_metadata"),
        "Expected Step 2 error, got: {:?}",
        err
    );
}

#[test]
fn a_secret_with_auth_method_none_still_means_client_secret_post() {
    // Figma echoes back the `"none"` it was sent while issuing a secret its
    // token endpoint then demands.
    let info: ClientInfo = serde_json::from_str(
        r#"{"client_id":"id","client_secret":"s","token_endpoint_auth_method":"none"}"#,
    )
    .unwrap();
    assert_eq!(info.auth_mode(), AuthMode::ClientSecretPost);
}

#[test]
fn no_secret_with_auth_method_none_stays_a_public_client() {
    let info: ClientInfo =
        serde_json::from_str(r#"{"client_id":"id","token_endpoint_auth_method":"none"}"#).unwrap();
    assert_eq!(info.auth_mode(), AuthMode::None);
}
