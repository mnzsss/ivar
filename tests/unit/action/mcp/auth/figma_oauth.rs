#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use crate::action::mcp::auth::{
    AuthMethod, Preregistration, ProviderRun,
    figma_oauth::{self, FlowOps},
    preregister::Preregistered,
};
use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::infra::figma::{self, OAuthEndpoints};
use crate::infra::http_callback::{AuthorizationCode, CallbackServer};
use crate::infra::oauth::{self, AuthMode, Tokens};
use crate::providers::Credential;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PipelineEvent {
    ConflictCheck,
    Preregister,
    Discover,
    Bind,
    OutputUrl,
    Wait,
    Exchange,
    Write,
    Verify,
}

#[test]
fn discover_oauth_endpoints_pure_parsing() {
    let header = r#"Bearer realm="Figma", resource_metadata="https://mcp.figma.com/.well-known/oauth-protected-resource""#;
    // Verify the parser extracts the resource_metadata URL from the header.
    assert_eq!(
        figma::parse_www_authenticate_resource_metadata(header),
        Some("https://mcp.figma.com/.well-known/oauth-protected-resource".to_owned())
    );
    let resource_json = r#"{"authorization_servers":["https://www.figma.com/oauth"],"resource":"https://api.figma.com","scopes_supported":["file_read"]}"#;
    let (authorization_server, resource) =
        figma::parse_resource_metadata(resource_json).expect("parse resource");
    assert_eq!(authorization_server, "https://www.figma.com/oauth");
    assert_eq!(resource, Some("https://api.figma.com".to_owned()));
    // Note: scopes_supported is not returned by parse_resource_metadata, it's in the auth metadata

    // And: fetching the authorization server metadata
    let auth_json = r#"{"authorization_endpoint":"https://www.figma.com/oauth/authorize","token_endpoint":"https://www.figma.com/oauth/token","scopes_supported":["file_read"]}"#;
    let endpoints = figma::parse_authorization_metadata(auth_json).expect("parse auth");
    assert_eq!(
        endpoints.authorization_endpoint,
        "https://www.figma.com/oauth/authorize"
    );
    assert_eq!(
        endpoints.token_endpoint,
        "https://www.figma.com/oauth/token"
    );
    assert_eq!(
        endpoints.scopes_supported,
        Some(vec!["file_read".to_owned()])
    );
}

#[test]
fn no_token_or_secret_appears_in_serialized_output() {
    let run = ProviderRun {
        provider: Provider::Omp,
        preregistration: Preregistration::Registered {
            client_id: "client-123".to_owned(),
        },
        auth_method: AuthMethod::InternalOAuthFlow,
        command: "ivar oauth".to_owned(),
        authenticated: true,
        error: None,
    };

    let debug_str = format!("{run:?}");
    let json_str = serde_json::to_string(&run).unwrap();

    let forbidden = [
        "secret",
        "token",
        "access_token",
        "refresh_token",
        "SUPER_SECRET",
    ];
    for term in forbidden {
        assert!(
            !debug_str.to_lowercase().contains(&term.to_lowercase()),
            "debug contained {term}"
        );
        assert!(
            !json_str.to_lowercase().contains(&term.to_lowercase()),
            "json contained {term}"
        );
    }
}

#[test]
fn provider_run_command_shows_internal_flow_label() {
    // Given: a ProviderRun from internal flow
    let run = ProviderRun {
        provider: Provider::OpenCode,
        preregistration: Preregistration::NotNeeded,
        auth_method: AuthMethod::InternalOAuthFlow,
        command: "ivar oauth".to_owned(),
        authenticated: true,
        error: None,
    };

    // Then: we can format it (Display impl should exist from mod.rs)
    let output = format!("{run:?}");
    assert!(output.contains("InternalOAuthFlow"));
    assert!(output.contains("ivar oauth"));
}

#[test]
fn provider_run_command_shows_provider_command_label() {
    // Given: a ProviderRun from provider-owned flow
    let run = ProviderRun {
        provider: Provider::OpenCode,
        preregistration: Preregistration::NotNeeded,
        auth_method: AuthMethod::ProviderCommand,
        command: "opencode mcp auth figma-test".to_owned(),
        authenticated: true,
        error: None,
    };

    // Then: we can format it
    let output = format!("{run:?}");
    assert!(output.contains("ProviderCommand"));
    assert!(output.contains("opencode mcp auth figma-test"));
}

struct MockOps {
    events: RefCell<Vec<PipelineEvent>>,
    conflict: bool,
    fail_at: Option<PipelineEvent>,
    written: RefCell<Option<(String, String, Option<String>, Tokens)>>,
    /// What step 2 hands back. Defaults to a client already registered with a
    /// secret; the regression test below swaps in what Figma returns.
    prereg: Option<Preregistered>,
    /// When set, `discover` reports this token endpoint and `exchange` performs
    /// the real [`oauth::exchange_code`] against it instead of faking success.
    token_endpoint: Option<String>,
    provider: Provider,
}

impl Default for MockOps {
    fn default() -> Self {
        Self {
            events: RefCell::new(Vec::new()),
            conflict: false,
            fail_at: None,
            written: RefCell::new(None),
            prereg: None,
            token_endpoint: None,
            provider: Provider::OpenCode,
        }
    }
}

impl FlowOps for MockOps {
    fn provider(&self) -> Provider {
        self.provider
    }
    fn check_conflict(&self, _: &str) -> Result<bool, Failure> {
        self.events.borrow_mut().push(PipelineEvent::ConflictCheck);
        if self.fail_at == Some(PipelineEvent::ConflictCheck) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(self.conflict)
    }
    fn preregister(
        &self,
        _: &McpServerDef,
        _: &str,
        _: &OAuthEndpoints,
    ) -> Result<Preregistered, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Preregister);
        if self.fail_at == Some(PipelineEvent::Preregister) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(match &self.prereg {
            Some(prereg) => Preregistered {
                report: prereg.report.clone(),
                client_id: prereg.client_id.clone(),
                secret: prereg.secret.clone(),
                auth_mode: prereg.auth_mode,
            },
            None => Preregistered {
                report: Preregistration::NotNeeded,
                client_id: Some("id".to_owned()),
                secret: Some(("VAR".to_owned(), "secret".to_owned())),
                auth_mode: AuthMode::ClientSecretPost,
            },
        })
    }
    fn discover(&self, _: &str) -> Result<OAuthEndpoints, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Discover);
        if self.fail_at == Some(PipelineEvent::Discover) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(OAuthEndpoints {
            authorization_endpoint: "a".to_owned(),
            token_endpoint: self
                .token_endpoint
                .clone()
                .unwrap_or_else(|| "t".to_owned()),
            resource: None,
            scopes_supported: None,
            registration_endpoint: None,
        })
    }
    fn bind(&self, _: &str) -> Result<CallbackServer, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Bind);
        if self.fail_at == Some(PipelineEvent::Bind) {
            return Err(Failure::failed("fail", "fail"));
        }
        CallbackServer::bind_on("state", Duration::from_secs(1))
    }
    fn output_url(&self, _: &str) {
        self.events.borrow_mut().push(PipelineEvent::OutputUrl);
    }
    fn wait_code(&self, _: CallbackServer) -> Result<AuthorizationCode, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Wait);
        if self.fail_at == Some(PipelineEvent::Wait) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(AuthorizationCode("code".to_owned()))
    }
    fn exchange(
        &self,
        endpoint: &str,
        code: &str,
        verifier: &str,
        id: &str,
        secret: Option<&str>,
        mode: AuthMode,
        resource: Option<&str>,
    ) -> Result<Tokens, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Exchange);
        if self.fail_at == Some(PipelineEvent::Exchange) {
            return Err(Failure::failed("fail", "fail"));
        }
        if self.token_endpoint.is_some() {
            return oauth::exchange_code(
                endpoint,
                code,
                "http://127.0.0.1:19876/callback",
                &oauth::CodeVerifier(verifier.to_owned()),
                id,
                secret,
                mode,
                resource,
            );
        }
        Ok(Tokens {
            access_token: "at".to_owned(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        })
    }
    fn write(&self, _: &str, credential: &Credential<'_>) -> Result<(), Failure> {
        self.events.borrow_mut().push(PipelineEvent::Write);
        *self.written.borrow_mut() = Some((
            credential.server_url.to_owned(),
            credential.client_id.to_owned(),
            credential.client_secret.map(str::to_owned),
            credential.tokens.clone(),
        ));
        if self.fail_at == Some(PipelineEvent::Write) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(())
    }
    fn verify(&self, _: &str, _: Option<&str>) -> Result<bool, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Verify);
        if self.fail_at == Some(PipelineEvent::Verify) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(true)
    }
}

#[test]
fn conflict_is_checked_before_any_side_effect() {
    let ops = MockOps {
        conflict: true,
        fail_at: None,
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert_eq!(*ops.events.borrow(), vec![PipelineEvent::ConflictCheck]);
}

#[test]
fn discovery_failure_does_not_write_credentials() {
    let ops = MockOps {
        conflict: false,
        fail_at: Some(PipelineEvent::Discover),
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_none());
}

#[test]
fn callback_failure_does_not_write_credentials() {
    let ops = MockOps {
        conflict: false,
        fail_at: Some(PipelineEvent::Wait),
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_none());
}

#[test]
fn exchange_failure_does_not_write_credentials() {
    let ops = MockOps {
        conflict: false,
        fail_at: Some(PipelineEvent::Exchange),
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_none());
}

#[test]
fn successful_flow_runs_in_contract_order() {
    let ops = MockOps {
        conflict: false,
        fail_at: None,
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert_eq!(
        *ops.events.borrow(),
        vec![
            PipelineEvent::ConflictCheck,
            PipelineEvent::Discover,
            PipelineEvent::Preregister,
            PipelineEvent::Bind,
            PipelineEvent::OutputUrl,
            PipelineEvent::Wait,
            PipelineEvent::Exchange,
            PipelineEvent::Write,
            PipelineEvent::Verify
        ]
    );
}

#[test]
fn internal_flow_threads_omp_provider_through_to_credential_installer() {
    let ops = MockOps {
        conflict: false,
        fail_at: None,
        provider: Provider::Omp,
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let run = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma").unwrap();
    assert_eq!(run.provider, Provider::Omp);
    assert_eq!(ops.provider(), Provider::Omp);
    assert!(ops.written.borrow().is_some());
}

#[test]
fn successful_flow_builds_complete_credential() {
    let ops = MockOps {
        conflict: false,
        fail_at: None,
        ..Default::default()
    };
    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_some());
    let (server_url, client_id, client_secret, tokens) = ops.written.borrow().clone().unwrap();
    assert_eq!(server_url, "https://mcp.figma.com/mcp");
    assert_eq!(client_id, "id");
    assert_eq!(client_secret, Some("secret".to_owned()));
    assert_eq!(tokens.access_token, "at");
}

// -- fresh-registration regression --------------------------------------

/// What Figma's registration endpoint returns (measured 2026-08-26):
/// a `client_secret` alongside `token_endpoint_auth_method: "none"`.
const FIGMA_REGISTRATION_RESPONSE: &str = r#"{
    "client_id": "VGup4YT70EEtoQUwR0OEwB",
    "client_secret": "the-secret",
    "token_endpoint_auth_method": "none"
}"#;

/// A one-shot stand-in for Figma's token endpoint: 400 `Client secret is
/// required` unless the form body carries a `client_secret`, matching the
/// real endpoint. Returns its URL.
fn figma_like_token_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());

        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if line.to_ascii_lowercase().starts_with("content-length:") {
                content_length = line.split(':').nth(1).unwrap().trim().parse().unwrap();
            }
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();

        let response = if body.contains("client_secret=") {
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"access_token\":\"at\"}"
        } else {
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n\r\n\
             {\"error\":\"Client secret is required\"}"
        };
        stream.write_all(response.as_bytes()).unwrap();
        stream.shutdown(std::net::Shutdown::Both).unwrap();
    });

    format!("http://127.0.0.1:{port}")
}

#[test]
fn fresh_figma_registration_sends_the_client_secret_to_the_token_endpoint() {
    let info: figma::ClientInfo = serde_json::from_str(FIGMA_REGISTRATION_RESPONSE).unwrap();
    let ops = MockOps {
        prereg: Some(Preregistered {
            report: Preregistration::Registered {
                client_id: info.client_id.clone(),
            },
            client_id: Some(info.client_id.clone()),
            secret: info
                .client_secret
                .clone()
                .map(|s| ("IVAR_MCP_FIGMA_SECRET".to_owned(), s)),
            auth_mode: info.auth_mode(),
        }),
        token_endpoint: Some(figma_like_token_endpoint()),
        ..Default::default()
    };

    let server = McpServerDef::new("figma", "http").url("https://mcp.figma.com/mcp");
    let result = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");

    assert!(
        result.is_ok(),
        "token exchange rejected: {:?}",
        result.err().map(|f| f.what)
    );
}
