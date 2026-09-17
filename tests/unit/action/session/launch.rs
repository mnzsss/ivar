//! Unit tests for `crate::action::session::launch`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
// `use super::*` already reaches `launch.rs`'s own imports — `Layout`,
// `Manifest`, `Provider`, `SessionId`, `FeatureName`, `Command`. Only what
// the tests add is imported here, so `clippy --all-targets -D warnings`
// sees no redundant import.
use crate::domain::mcp::McpServerDef;
use crate::domain::name::HallName;
use crate::store::manifest::Providers;
use camino::Utf8PathBuf;

fn manifest_with_graph_server() -> Manifest {
    Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap()
    .with_mcp_servers(vec![
        McpServerDef::new("graph", "local")
            .command("ivar")
            .args(vec!["graph".to_owned(), "mcp".to_owned()]),
    ])
    .unwrap()
}

#[test]
fn a_claude_launch_carries_the_halls_mcp_allowlist() {
    let layout = Layout::at(Utf8PathBuf::from("/tmp/acme"));
    let manifest = manifest_with_graph_server();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);

    let command = provider_command(
        &layout,
        &manifest,
        Provider::ClaudeCode,
        &view_dir,
        &session_id,
        None,
        false,
        &[],
    )
    .expect("a claude launch must build");

    assert_eq!(command.program(), "claude");
    let args = command.arguments();
    let settings_at = args
        .iter()
        .position(|arg| arg == "--settings")
        .expect("claude must receive process-scoped settings");
    assert_eq!(
        args[settings_at + 1],
        r#"{"enabledMcpjsonServers":["acme-graph"]}"#
    );
}

#[test]
fn a_launch_runs_in_the_view_dir_with_the_session_environment() {
    let layout = Layout::at(Utf8PathBuf::from("/tmp/acme"));
    let manifest = manifest_with_graph_server();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let feature = FeatureName::new("checkout").unwrap();
    let view_dir = layout.feature_session(&feature, &session_id);

    let command = provider_command(
        &layout,
        &manifest,
        Provider::ClaudeCode,
        &view_dir,
        &session_id,
        Some(&feature),
        false,
        &[],
    )
    .expect("a feature-session launch must build");

    let envs: Vec<(&str, &str)> = command
        .envs()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert!(envs.contains(&("IVAR_SESSION_PATH", view_dir.as_str())));
    assert!(envs.contains(&("IVAR_HALL", "/tmp/acme")));
    assert!(envs.contains(&("IVAR_PROVIDER", "claude-code")));
    assert!(envs.contains(&("IVAR_FEATURE", "checkout")));
    assert_eq!(command.working_dir(), Some(view_dir.as_path()));
}

#[test]
fn caller_arguments_come_after_the_generated_ones() {
    let layout = Layout::at(Utf8PathBuf::from("/tmp/acme"));
    let manifest = manifest_with_graph_server();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);

    let command = provider_command(
        &layout,
        &manifest,
        Provider::ClaudeCode,
        &view_dir,
        &session_id,
        None,
        false,
        &["-p".to_owned(), "explain normalizeLedgerEntry".to_owned()],
    )
    .expect("a print-mode launch must build");

    let args = command.arguments();
    let settings_at = args.iter().position(|arg| arg == "--settings").unwrap();
    let print_at = args.iter().position(|arg| arg == "-p").unwrap();
    assert!(
        settings_at < print_at,
        "the caller's arguments must win a repeated flag: {args:?}"
    );
    assert_eq!(
        args.last().map(String::as_str),
        Some("explain normalizeLedgerEntry")
    );
}

#[test]
fn opencode_and_omp_keep_their_resume_argument() {
    let layout = Layout::at(Utf8PathBuf::from("/tmp/acme"));
    let manifest = manifest_with_graph_server();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);

    for (provider, binary) in [(Provider::OpenCode, "opencode"), (Provider::Omp, "omp")] {
        let command = provider_command(
            &layout,
            &manifest,
            provider,
            &view_dir,
            &session_id,
            None,
            true,
            &[],
        )
        .expect("a resumed launch must build");

        assert_eq!(command.program(), binary);
        assert_eq!(command.arguments(), &["--continue"]);
    }
}

#[test]
fn a_program_that_is_not_the_session_provider_is_refused() {
    let error = ensure_provider_binary(Provider::ClaudeCode, "opencode")
        .expect_err("the launcher runs the session's provider, nothing else");
    assert_eq!(error.code, "sandbox.launcher_foreign_program");

    ensure_provider_binary(Provider::ClaudeCode, "claude")
        .expect("the session provider's own binary is accepted");
}
