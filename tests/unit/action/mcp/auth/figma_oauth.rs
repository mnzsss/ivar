#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::time::Duration;

use crate::action::mcp::auth::{
    AuthMethod, Preregistration, ProviderRun,
    figma_oauth::{self, FlowOps},
    preregister::Preregistered,
};
use crate::domain::mcp::McpServerDef;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::harness::opencode_auth::Entry;
use crate::infra::figma::{self, OAuthEndpoints};
use crate::infra::http_callback::{AuthorizationCode, CallbackServer};
use crate::infra::oauth::{AuthMode, Tokens};

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
    written: RefCell<Option<Entry>>,
}

impl FlowOps for MockOps {
    fn check_conflict(&self, _: &str) -> Result<bool, Failure> {
        self.events.borrow_mut().push(PipelineEvent::ConflictCheck);
        if self.fail_at == Some(PipelineEvent::ConflictCheck) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(self.conflict)
    }
    fn preregister(&self, _: &McpServerDef, _: &str) -> Result<Preregistered, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Preregister);
        if self.fail_at == Some(PipelineEvent::Preregister) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(Preregistered {
            report: Preregistration::NotNeeded,
            client_id: Some("id".to_owned()),
            secret: Some(("VAR".to_owned(), "secret".to_owned())),
            auth_mode: AuthMode::ClientSecretPost,
        })
    }
    fn discover(&self, _: &str) -> Result<OAuthEndpoints, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Discover);
        if self.fail_at == Some(PipelineEvent::Discover) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(OAuthEndpoints {
            authorization_endpoint: "a".to_owned(),
            token_endpoint: "t".to_owned(),
            resource: None,
            scopes_supported: None,
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
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: AuthMode,
        _: Option<&str>,
    ) -> Result<Tokens, Failure> {
        self.events.borrow_mut().push(PipelineEvent::Exchange);
        if self.fail_at == Some(PipelineEvent::Exchange) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(Tokens {
            access_token: "at".to_owned(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        })
    }
    fn write(&self, _: &str, entry: &Entry) -> Result<(), Failure> {
        self.events.borrow_mut().push(PipelineEvent::Write);
        *self.written.borrow_mut() = Some(entry.clone());
        if self.fail_at == Some(PipelineEvent::Write) {
            return Err(Failure::failed("fail", "fail"));
        }
        Ok(())
    }
    fn verify(&self, _: &str) -> Result<bool, Failure> {
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
        events: RefCell::new(Vec::new()),
        conflict: true,
        fail_at: None,
        written: RefCell::new(None),
    };
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert_eq!(*ops.events.borrow(), vec![PipelineEvent::ConflictCheck]);
}

#[test]
fn discovery_failure_does_not_write_credentials() {
    let ops = MockOps {
        events: RefCell::new(Vec::new()),
        conflict: false,
        fail_at: Some(PipelineEvent::Discover),
        written: RefCell::new(None),
    };
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_none());
}

#[test]
fn callback_failure_does_not_write_credentials() {
    let ops = MockOps {
        events: RefCell::new(Vec::new()),
        conflict: false,
        fail_at: Some(PipelineEvent::Wait),
        written: RefCell::new(None),
    };
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_none());
}

#[test]
fn exchange_failure_does_not_write_credentials() {
    let ops = MockOps {
        events: RefCell::new(Vec::new()),
        conflict: false,
        fail_at: Some(PipelineEvent::Exchange),
        written: RefCell::new(None),
    };
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_none());
}

#[test]
fn successful_flow_runs_in_contract_order() {
    let ops = MockOps {
        events: RefCell::new(Vec::new()),
        conflict: false,
        fail_at: None,
        written: RefCell::new(None),
    };
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert_eq!(
        *ops.events.borrow(),
        vec![
            PipelineEvent::ConflictCheck,
            PipelineEvent::Preregister,
            PipelineEvent::Discover,
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
fn successful_flow_builds_complete_opencode_entry() {
    let ops = MockOps {
        events: RefCell::new(Vec::new()),
        conflict: false,
        fail_at: None,
        written: RefCell::new(None),
    };
    let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
    let _ = figma_oauth::run_internal_flow_pipeline(&ops, &server, "figma");
    assert!(ops.written.borrow().is_some());
    let written = ops.written.borrow().clone().unwrap();
    assert_eq!(written.server_url, "https://mcp.figma.com/mcp");
    assert_eq!(written.client_info.client_id, "id");
    assert_eq!(written.client_info.client_secret, Some("secret".to_owned()));
    assert_eq!(written.tokens.access_token, "at");
}
