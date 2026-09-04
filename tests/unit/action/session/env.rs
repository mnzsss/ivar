#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::store::layout::Layout;

#[test]
fn session_env_renders_shell_and_json_and_applies_to_command() {
    let hall = Utf8PathBuf::from("/tmp/acme");
    let layout = Layout::at(hall.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = Utf8PathBuf::from(
        "/tmp/acme/.ivar/features/checkout/sessions/6f0c9d5f-0000-4000-8000-000000000000",
    );
    let feature = FeatureName::new("checkout").unwrap();
    let env = SessionEnv::build(
        &layout,
        &session_id,
        &view_dir,
        Provider::ClaudeCode,
        Some(&feature),
    );

    let shell = env.render_shell();
    assert!(shell.contains(&format!("export IVAR_HALL={hall}")));
    assert!(shell.contains("export IVAR_SESSION_ID=6f0c9d5f-0000-4000-8000-000000000000"));
    assert!(shell.contains(&format!("export IVAR_SESSION_PATH={view_dir}")));
    assert!(shell.contains("export IVAR_PROVIDER=claude-code"));
    assert!(shell.contains("export IVAR_FEATURE=checkout"));

    let json = env.render_json();
    assert_eq!(
        json["IVAR_SESSION_ID"],
        "6f0c9d5f-0000-4000-8000-000000000000"
    );
    assert_eq!(json["IVAR_PROVIDER"], "claude-code");

    let command = env.apply(crate::infra::proc::Command::new("sh"));
    let envs = command.envs();
    assert!(
        envs.iter()
            .any(|(k, v)| k == "IVAR_SESSION_ID" && v == "6f0c9d5f-0000-4000-8000-000000000000")
    );

    // A discovery session carries no feature.
    let discovery_dir = layout.discovery_session(&session_id);
    let discovery = SessionEnv::build(
        &layout,
        &session_id,
        &discovery_dir,
        Provider::ClaudeCode,
        None,
    );
    assert!(!discovery.render_shell().contains("IVAR_FEATURE"));
    assert!(discovery.render_json().get("IVAR_FEATURE").is_none());
}

#[test]
fn omp_session_env_resolves_all_five_variables() {
    let hall = Utf8PathBuf::from("/tmp/acme");
    let layout = Layout::at(hall.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = Utf8PathBuf::from(
        "/tmp/acme/.ivar/features/checkout/sessions/6f0c9d5f-0000-4000-8000-000000000000",
    );
    let feature = FeatureName::new("checkout").unwrap();
    let env = SessionEnv::build(
        &layout,
        &session_id,
        &view_dir,
        Provider::Omp,
        Some(&feature),
    );

    let shell = env.render_shell();
    assert!(shell.contains(&format!("export IVAR_HALL={hall}")));
    assert!(shell.contains("export IVAR_SESSION_ID=6f0c9d5f-0000-4000-8000-000000000000"));
    assert!(shell.contains(&format!("export IVAR_SESSION_PATH={view_dir}")));
    assert!(shell.contains("export IVAR_PROVIDER=omp"));
    assert!(shell.contains("export IVAR_FEATURE=checkout"));

    let json = env.render_json();
    assert_eq!(json["IVAR_HALL"], "/tmp/acme");
    assert_eq!(
        json["IVAR_SESSION_ID"],
        "6f0c9d5f-0000-4000-8000-000000000000"
    );
    assert_eq!(json["IVAR_SESSION_PATH"], view_dir.as_str());
    assert_eq!(json["IVAR_PROVIDER"], "omp");
    assert_eq!(json["IVAR_FEATURE"], "checkout");

    let command = env.apply(crate::infra::proc::Command::new("omp"));
    let envs: std::collections::HashMap<_, _> = command
        .envs()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(envs.get("IVAR_HALL").copied(), Some("/tmp/acme"));
    assert_eq!(
        envs.get("IVAR_SESSION_ID").copied(),
        Some("6f0c9d5f-0000-4000-8000-000000000000")
    );
    assert_eq!(
        envs.get("IVAR_SESSION_PATH").copied(),
        Some(view_dir.as_str())
    );
    assert_eq!(envs.get("IVAR_PROVIDER").copied(), Some("omp"));
    assert_eq!(envs.get("IVAR_FEATURE").copied(), Some("checkout"));
}
