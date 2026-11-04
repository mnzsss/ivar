//! `ivar memory capture`: turns one provider hook payload into a sanitized
//! Observation appended to the session's spool file. It never opens the memory
//! database and swallows every failure so an agent is never blocked by it.

use camino::{Utf8Path, Utf8PathBuf};

use super::classify;
use crate::domain::observation::{Importance, Observation, SourceEvent, event_id};
use crate::domain::provider::Provider;
use crate::domain::sanitize::{SanitizeContext, Sanitized};
use crate::domain::session::rfc3339_now;
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
    let line = serde_json::to_string(sanitized.as_observation()).ok()?;
    let key = SpoolKey::new(env.session_id.as_deref(), &event.native_session_id);
    spool::append(&layout.memory_spool_dir(), &key, &line).ok()?;
    Some(())
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
