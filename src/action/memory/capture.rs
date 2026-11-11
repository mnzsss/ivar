//! `ivar memory capture`: turns one provider hook payload into a sanitized
//! Observation appended to the session's spool file. It never opens the memory
//! database and swallows every failure so an agent is never blocked by it.
//!
//! The one thing it does besides spooling: when a session that has no feature
//! ends, it starts `ivar hall sync` in a process of its own and returns without
//! waiting, so the pages that session wrote outside any feature reach the
//! hall's history even if nobody ever runs `ivar session stop`.

use camino::{Utf8Path, Utf8PathBuf};

use super::classify;
use crate::action::read_manifest;
use crate::domain::observation::{Importance, Observation, SourceEvent, event_id};
use crate::domain::provider::Provider;
use crate::domain::sanitize::{SanitizeContext, Sanitized};
use crate::domain::session::rfc3339_now;
use crate::infra::proc;
use crate::providers::capture::{CaptureEvent, CaptureKind, parse_capture};
use crate::store::layout::Layout;
use crate::store::memory::spool::{self, SpoolKey};

const MIN_REDACTED_ENV_VALUE_CHARS: usize = 6;

#[derive(Debug, Default)]
pub struct CaptureEnv {
    pub hall: Option<Utf8PathBuf>,
    pub session_id: Option<String>,
    pub provider: Option<String>,
    pub feature: Option<String>,
    pub session_path: Option<Utf8PathBuf>,
}

impl CaptureEnv {
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            hall: var("IVAR_HALL").map(Utf8PathBuf::from),
            session_id: var("IVAR_SESSION_ID"),
            provider: var("IVAR_PROVIDER"),
            feature: var("IVAR_FEATURE"),
            session_path: var("IVAR_SESSION_PATH").map(Utf8PathBuf::from),
        }
    }
}

pub fn capture(provider: Provider, stdin: &str, env: &CaptureEnv) {
    let _ = try_capture(provider, stdin, env);
}

fn try_capture(provider: Provider, stdin: &str, env: &CaptureEnv) -> Option<()> {
    let event = parse_capture(provider, stdin).ok()?;
    let layout = match &env.hall {
        Some(hall) => Layout::at(hall.clone()),
        None => Layout::discover(event.cwd.as_deref()?).ok()??,
    };
    let observation = build_observation(provider, &event, env, &layout);
    let sanitized = Sanitized::sanitize(observation, &sanitize_context(&layout)).ok()?;
    let key = SpoolKey::new(env.session_id.as_deref(), &event.native_session_id);
    spool::append(&layout.memory_spool_dir(), &key, &sanitized).ok()?;
    if matches!(event.kind, CaptureKind::Ended { .. }) && env.feature.is_none() {
        sync_hall_detached(&layout);
    }
    Some(())
}

/// The program that runs `ivar hall sync`: this very binary.
///
/// A test build never names itself: it would run the test harness. It names a
/// program that does not exist, which a test redirects to a fake of its own.
#[cfg(not(test))]
fn ivar_program() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps)]
fn ivar_program() -> Option<String> {
    Some(TEST_IVAR_PROGRAM.to_owned())
}

/// What a test build runs in place of `ivar`; see [`ivar_program`].
#[cfg(test)]
pub(crate) const TEST_IVAR_PROGRAM: &str = "ivar-under-test";

/// Start `ivar hall sync` for the hall, detached, its output going to the
/// hall's sync log, and do not wait for it.
///
/// Nothing here can fail the capture: a hall that turns `auto_commit` off, a
/// manifest that cannot be read, a binary that cannot be found or spawned all
/// end in silence. The child is a manual sync, which ignores `auto_commit`, so
/// the switch is checked here.
fn sync_hall_detached(layout: &Layout) {
    if !read_manifest(layout).is_ok_and(|manifest| manifest.auto_commit()) {
        return;
    }
    let Some(program) = ivar_program() else {
        return;
    };
    let command = proc::Command::new(program)
        .args(["hall", "sync"])
        .cwd(layout.root());
    let _ = proc::detach_to_log(&command, &layout.hall_sync_log());
}

fn build_observation(
    provider: Provider,
    event: &CaptureEvent,
    env: &CaptureEnv,
    layout: &Layout,
) -> Observation {
    let (source_event, origin, importance, body) = match &event.kind {
        CaptureKind::Started => (
            SourceEvent::SessionStarted,
            None,
            Importance::session_started(),
            "{}".to_owned(),
        ),
        CaptureKind::Prompted { text } => {
            let origin = classify::prompt_origin(text);
            (
                SourceEvent::SessionPrompted,
                Some(origin),
                Importance::session_prompted(origin),
                text.clone(),
            )
        }
        CaptureKind::Tool {
            name,
            input,
            failed,
            result,
        } => {
            let effect = classify::tool_effect(name, input, *failed);
            let body = classify::tool_body(name, input, effect, result.as_deref());
            (
                SourceEvent::SessionTool,
                None,
                Importance::session_tool(effect),
                body,
            )
        }
        CaptureKind::Ended { reason } => (
            SourceEvent::SessionEnded,
            None,
            Importance::session_ended(),
            serde_json::json!({ "reason": reason }).to_string(),
        ),
    };
    let occurred_at = rfc3339_now();
    let feature = env.feature.clone().unwrap_or_default();
    Observation {
        event_id: event_id(
            &event.native_session_id,
            source_event,
            &occurred_at,
            std::process::id(),
        ),
        source_event: source_event.as_str().to_owned(),
        occurred_at,
        session_id: env.session_id.clone().unwrap_or_default(),
        native_session_id: event.native_session_id.clone(),
        provider: env
            .provider
            .clone()
            .unwrap_or_else(|| provider.id().to_owned()),
        ivar_version: env!("CARGO_PKG_VERSION").to_owned(),
        parent_feature: parent_feature(layout, &feature),
        repo: repo_of(event.cwd.as_deref(), env.session_path.as_deref()),
        feature,
        origin: origin.map_or_else(String::new, |o| o.as_str().to_owned()),
        importance,
        attempt_id: String::new(),
        body,
    }
}

fn repo_of(cwd: Option<&Utf8Path>, session_path: Option<&Utf8Path>) -> String {
    let relative = cwd
        .zip(session_path)
        .and_then(|(cwd, view)| cwd.strip_prefix(view).ok());
    relative
        .and_then(|path| path.components().next())
        .map_or_else(String::new, |first| first.as_str().to_owned())
}

fn parent_feature(layout: &Layout, feature: &str) -> String {
    let path = layout
        .ivar_dir()
        .join("features")
        .join(feature)
        .join("feature.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|doc| doc.get("parent")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub(crate) fn sanitize_context(layout: &Layout) -> SanitizeContext {
    let values = std::fs::read_to_string(layout.root().join(".env"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(_, value)| value.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|value| value.chars().count() >= MIN_REDACTED_ENV_VALUE_CHARS)
        .collect();
    SanitizeContext::new(layout.root().to_path_buf(), values)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/memory/capture.rs"]
mod tests;
