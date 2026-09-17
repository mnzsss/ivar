//! The session's provider launch, as one value.
//!
//! `ivar` launches a provider from two places: the TUI on an interactive
//! `session start`, and the hidden `ivar session sandbox` launcher a
//! detached session is started through (ADR-0006 D1). Both must produce the
//! same invocation — provider-native arguments, the session environment,
//! MCP secrets, and the view dir as the working directory — so it is built
//! here, once, from the session's record on disk.
//!
//! Not to be confused with `crate::providers::*::launch`, which owns one
//! provider's argv. This module composes that with the session's own facts.

use camino::Utf8Path;

use crate::action::session::env::SessionEnv;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::proc::Command;
use crate::providers;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// The provider invocation for one session.
///
/// `user_args` are appended after the generated arguments: a caller that
/// repeats a flag means to override, and every provider CLI here takes the
/// last occurrence.
#[allow(clippy::too_many_arguments)]
pub(crate) fn provider_command(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    view_dir: &Utf8Path,
    session_id: &SessionId,
    feature: Option<&FeatureName>,
    resume: bool,
    user_args: &[String],
) -> Result<Command, Failure> {
    let hall_name = manifest.name().clone();
    let mut allowlist: Vec<String> = manifest
        .mcp_servers()
        .iter()
        .map(|server| server.materialised_name(&hall_name))
        .collect();
    allowlist.sort();
    allowlist.dedup();

    let command = providers::start_command(provider, resume, &allowlist)?;
    let command = command.args(user_args.iter().cloned());
    let command = SessionEnv::build(layout, session_id, view_dir, provider, feature).apply(command);
    let command =
        crate::action::mcp::inject_session_mcp_secrets(command, layout, manifest, provider);

    Ok(command.cwd(view_dir.to_path_buf()))
}

/// Refuse a program that is not this session's provider.
///
/// The launcher rebuilds the invocation from the session's provider, so an
/// argv naming anything else has no meaning: there is no contract to
/// rebuild for `ls`.
pub(crate) fn ensure_provider_binary(provider: Provider, program: &str) -> Result<(), Failure> {
    let expected = providers::launch_contract(provider).binary;
    if program == expected {
        return Ok(());
    }
    Err(Failure::blocked(
        "sandbox.launcher_foreign_program",
        format!("`{program}` is not the provider this session runs"),
    )
    .expected(format!("`{expected}`, the session provider's binary"))
    .actual(format!("`{program}`"))
    .fix(FixAction::safe(
        "session.launch_session_provider",
        format!(
            "Run `{expected}` through the launcher, or start a session on the provider you want."
        ),
    )))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/launch.rs"]
mod tests;
