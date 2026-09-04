#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

use super::*;
use crate::infra::oauth::AuthMode;

#[test]
fn discovery_failures_are_coded_to_the_generic_module_not_the_vendor() {
    let malformed = "{not json";
    let failure =
        parse_authorization_metadata(malformed).expect_err("malformed metadata must fail");
    assert_eq!(failure.code, "mcp_oauth.auth_metadata_parse");
    assert!(
        !failure.code.starts_with("figma."),
        "a generic discovery failure must not be reported as a Figma failure"
    );
}

#[test]
fn resource_metadata_without_an_authorization_server_is_coded_generically() {
    let empty = r#"{"authorization_servers": []}"#;
    let failure =
        parse_resource_metadata(empty).expect_err("an empty authorization_servers array must fail");
    assert_eq!(
        failure.code,
        "mcp_oauth.resource_metadata_no_authorization_server"
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
        err.code.contains("mcp_oauth.discover_resource_metadata"),
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

#[test]
fn authorization_metadata_carries_the_registration_endpoint() {
    let json = r#"{
        "issuer": "https://mcp.linear.app",
        "authorization_endpoint": "https://mcp.linear.app/authorize",
        "token_endpoint": "https://mcp.linear.app/token",
        "registration_endpoint": "https://mcp.linear.app/register",
        "scopes_supported": ["read", "write"]
    }"#;
    let endpoints = parse_authorization_metadata(json).expect("valid RFC 8414 metadata");
    assert_eq!(
        endpoints.registration_endpoint.as_deref(),
        Some("https://mcp.linear.app/register")
    );
}

#[test]
fn authorization_metadata_without_a_registration_endpoint_is_still_valid() {
    let json = r#"{
        "issuer": "https://mcp.figma.com",
        "authorization_endpoint": "https://mcp.figma.com/authorize",
        "token_endpoint": "https://mcp.figma.com/token"
    }"#;
    let endpoints = parse_authorization_metadata(json).expect("registration_endpoint is optional");
    assert_eq!(endpoints.registration_endpoint, None);
}

#[test]
fn a_registration_without_a_secret_is_a_public_client() {
    let body = r#"{
        "client_id": "Uk1eiX5O6ndVHwo_",
        "redirect_uris": ["http://127.0.0.1:19876/callback"],
        "token_endpoint_auth_method": "none",
        "client_id_issued_at": 1788544707
    }"#;
    let info = parse_registration_response(body).expect("a valid RFC 7591 response");
    assert_eq!(info.client_id, "Uk1eiX5O6ndVHwo_");
    assert_eq!(info.client_secret, None);
    assert_eq!(info.auth_mode(), crate::infra::oauth::AuthMode::None);
}

#[test]
fn a_registration_carrying_a_secret_is_confidential_even_when_it_says_none() {
    let body = r#"{
        "client_id": "abc",
        "client_secret": "shh",
        "token_endpoint_auth_method": "none"
    }"#;
    let info = parse_registration_response(body).expect("a valid RFC 7591 response");
    assert_eq!(
        info.auth_mode(),
        crate::infra::oauth::AuthMode::ClientSecretPost,
        "a response carrying a secret is confidential regardless of what it claims"
    );
}

#[test]
fn a_registration_response_that_is_not_json_names_the_generic_module() {
    let failure = parse_registration_response("<html>502</html>")
        .expect_err("HTML is not a registration response");
    assert_eq!(failure.code, "mcp_oauth.register_client_parse");
}
