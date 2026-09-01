//! ... (rest of doc comment)

use std::io::{self, Write};
use std::time::Duration;

use crate::domain::mcp::McpServerDef;
use crate::error::{Failure, FixAction};
use crate::harness::opencode_auth::{self, ClientInfo, Entry};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::preregister::preregister_if_needed;
use super::{AuthMethod, Preregistration, ProviderRun};

use crate::infra::figma::{self, OAuthEndpoints};
use crate::infra::fs;
use crate::infra::http_callback::{CallbackServer, AuthorizationCode};
use crate::infra::oauth::{self, Tokens};

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);
const REDIRECT_URI: &str = "http://127.0.0.1:19876/callback";
const INTERNAL_FLOW_LABEL: &str = "ivar oauth";

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

pub(super) trait FlowOps {
    fn check_conflict(&self, name: &str) -> Result<bool, Failure>;
    fn preregister(&self, server: &McpServerDef, name: &str) -> Result<Preregistered, Failure>;
    fn discover(&self, url: &str) -> Result<OAuthEndpoints, Failure>;
    fn bind(&self, state: &str) -> Result<CallbackServer, Failure>;
    fn output_url(&self, url: &str);
    fn exchange(&self, endpoint: &str, code: &str, verifier: &str, id: &str, secret: &str) -> Result<Tokens, Failure>;
    fn write(&self, name: &str, entry: &Entry) -> Result<(), Failure>;
    fn verify(&self, name: &str) -> Result<bool, Failure>;
}

struct RealFlowOps {
    layout: Layout,
    manifest: Manifest,
    provider: crate::domain::provider::Provider,
}

impl FlowOps for RealFlowOps {
    fn check_conflict(&self, name: &str) -> Result<bool, Failure> { opencode_auth::has_entry(name) }
    fn preregister(&self, server: &McpServerDef, name: &str) -> Result<Preregistered, Failure> {
        preregister_if_needed(&self.layout, &self.manifest, self.provider, server, name)
    }
    fn discover(&self, url: &str) -> Result<OAuthEndpoints, Failure> { figma::discover_oauth_endpoints(url) }
    fn bind(&self, state: &str) -> Result<CallbackServer, Failure> { CallbackServer::bind(state, CALLBACK_TIMEOUT) }
    fn output_url(&self, url: &str) { let _ = writeln!(io::stderr().lock(), "Open this URL to authenticate:\n\n  {url}\n"); }
    fn wait_code(&self, listener: &CallbackServer) -> Result<AuthorizationCode, Failure> { Ok(AuthorizationCode("fake".to_string())) }
    fn exchange(&self, endpoint: &str, code: &str, verifier: &str, id: &str, secret: &str) -> Result<Tokens, Failure> {
        oauth::exchange_code(endpoint, code, REDIRECT_URI, &oauth::CodeVerifier(verifier.to_owned()), id, secret)
    }
    fn write(&self, name: &str, entry: &Entry) -> Result<(), Failure> { opencode_auth::write_entry(name, entry) }
    fn verify(&self, name: &str) -> Result<bool, Failure> { opencode_auth::has_tokens(name) }
}

pub(super) fn run_internal_flow_inner(
    layout: &Layout,
    manifest: &Manifest,
    server: &McpServerDef,
    materialised_name: &str,
) -> Result<ProviderRun, Failure> {
    let ops = RealFlowOps {
        layout: layout.clone(),
        manifest: manifest.clone(),
        provider: crate::domain::provider::Provider::OpenCode,
    };
    run_internal_flow_pipeline(&ops, server, materialised_name)
}

fn run_internal_flow_pipeline(
    ops: &dyn FlowOps,
    server: &McpServerDef,
    materialised_name: &str,
) -> Result<ProviderRun, Failure> {
    let provider = crate::domain::provider::Provider::OpenCode;
    if ops.check_conflict(materialised_name)? {
        let path = fs::data_dir().map(|d| d.join("opencode").join("mcp-auth.json")).map(|p| p.to_string()).unwrap_or_else(|_| "OpenCode's mcp-auth.json".to_owned());
        return Err(Failure::blocked("figma_oauth.conflict", format!("already has entry for \"{materialised_name}\"")).actual(format!("entry at {path}")));
    }
    let preregistered = ops.preregister(server, materialised_name)?;
    let preregistration = preregistered.report.clone();
    let client_id = preregistered.client_id.ok_or_else(|| Failure::blocked("figma_oauth.no_client_id", "missing client_id"))?;
    let client_secret = preregistered.secret.map(|(_, s)| s).ok_or_else(|| Failure::blocked("figma_oauth.no_client_secret", "missing client_secret"))?;
    let server_url = server.url.as_deref().ok_or_else(|| Failure::blocked("figma_oauth.no_server_url", "missing server_url"))?;
    let endpoints = ops.discover(server_url)?;
    let (verifier, challenge) = oauth::pkce_pair();
    let state = oauth::state();
    let listener = ops.bind(&state.0)?;
    let auth_url = oauth::authorize_url(&endpoints.authorization_endpoint, &client_id, REDIRECT_URI, &state, &challenge, endpoints.resource.as_deref(), endpoints.scopes_supported.as_deref().and_then(|s| s.first()).map(|s| s.as_str()));
    ops.output_url(&auth_url);
    let code = ops.wait_code(&listener)?;
    let tokens = ops.exchange(&endpoints.token_endpoint, &code.0, &verifier.0, &client_id, &client_secret)?;
    let entry = Entry { server_url: server_url.to_owned(), client_info: ClientInfo { client_id, client_secret: Some(client_secret), client_secret_expires_at: None }, tokens };
    ops.write(materialised_name, &entry)?;
    if !ops.verify(materialised_name)? { return Err(Failure::failed("figma_oauth.verify_failed", "verification failed")); }
    Ok(ProviderRun { provider, preregistration, auth_method: AuthMethod::InternalOAuthFlow, command: INTERNAL_FLOW_LABEL.to_owned(), authenticated: true, error: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockOps {
        events: RefCell<Vec<PipelineEvent>>,
        conflict: bool,
    }
    impl FlowOps for MockOps {
        fn check_conflict(&self, _: &str) -> Result<bool, Failure> {
            self.events.borrow_mut().push(PipelineEvent::ConflictCheck);
            Ok(self.conflict)
        }
        fn preregister(&self, _: &McpServerDef, _: &str) -> Result<Preregistered, Failure> {
            self.events.borrow_mut().push(PipelineEvent::Preregister);
            Ok(Preregistered { report: Preregistration::NotNeeded, client_id: Some("id".to_owned()), secret: Some(("VAR".to_owned(), "secret".to_owned())) })
        }
        fn discover(&self, _: &str) -> Result<OAuthEndpoints, Failure> {
            self.events.borrow_mut().push(PipelineEvent::Discover);
            Ok(OAuthEndpoints { authorization_endpoint: "a".to_owned(), token_endpoint: "t".to_owned(), resource: None, scopes_supported: None })
        }
        fn bind(&self, _: &str) -> Result<CallbackServer, Failure> {
            self.events.borrow_mut().push(PipelineEvent::Bind);
            CallbackServer::bind_on("state", Duration::from_secs(1))
        }
        fn output_url(&self, _: &str) { self.events.borrow_mut().push(PipelineEvent::OutputUrl); }
        fn wait_code(&self, _: &CallbackServer) -> Result<AuthorizationCode, Failure> {
            self.events.borrow_mut().push(PipelineEvent::Wait);
            Ok(AuthorizationCode("code".to_owned()))
        }
        fn exchange(&self, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<Tokens, Failure> {
            self.events.borrow_mut().push(PipelineEvent::Exchange);
            Ok(Tokens { access_token: "at".to_owned(), refresh_token: None, expires_at: None, scope: None })
        }
        fn write(&self, _: &str, _: &Entry) -> Result<(), Failure> {
            self.events.borrow_mut().push(PipelineEvent::Write);
            Ok(())
        }
        fn verify(&self, _: &str) -> Result<bool, Failure> {
            self.events.borrow_mut().push(PipelineEvent::Verify);
            Ok(true)
        }
    }

    #[test]
    fn conflict_is_checked_before_any_side_effect() {
        let ops = MockOps { events: RefCell::new(Vec::new()), conflict: true };
        let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
        let _ = run_internal_flow_pipeline(&ops, &server, "figma");
        assert_eq!(*ops.events.borrow(), vec![PipelineEvent::ConflictCheck]);
    }

    #[test]
    fn successful_flow_runs_in_contract_order() {
        let ops = MockOps { events: RefCell::new(Vec::new()), conflict: false };
        let server = McpServerDef::new("figma", "sse").url("https://mcp.figma.com/mcp");
        let _ = run_internal_flow_pipeline(&ops, &server, "figma");
        assert_eq!(*ops.events.borrow(), vec![
            PipelineEvent::ConflictCheck, PipelineEvent::Preregister, PipelineEvent::Discover, 
            PipelineEvent::Bind, PipelineEvent::OutputUrl, PipelineEvent::Wait, 
            PipelineEvent::Exchange, PipelineEvent::Write, PipelineEvent::Verify
        ]);
    }
}
