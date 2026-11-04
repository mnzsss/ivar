//! Provider-native session hook payloads parsed into one capture event shape.

use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::domain::provider::Provider;
use crate::providers::{claude_code, omp, opencode};

#[derive(Debug, PartialEq)]
pub enum CaptureKind {
    Started,
    Prompted {
        text: String,
    },
    Tool {
        name: String,
        input: serde_json::Value,
        failed: bool,
        result: Option<String>,
    },
    Ended {
        reason: Option<String>,
    },
}

#[derive(Debug, PartialEq)]
pub struct CaptureEvent {
    pub kind: CaptureKind,
    pub native_session_id: String,
    pub cwd: Option<Utf8PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseSkip(pub &'static str);

/// # Errors
///
/// Returns [`ParseSkip`] for a malformed payload, a missing field or an unsupported event.
pub fn parse_capture(provider: Provider, stdin: &str) -> Result<CaptureEvent, ParseSkip> {
    match provider {
        Provider::ClaudeCode | Provider::T3 => claude_code::capture::parse(stdin),
        Provider::OpenCode => opencode::capture::parse(stdin),
        Provider::Omp => omp::capture::parse(stdin),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct WireCapture {
    event: Option<String>,
    session: Option<String>,
    cwd: Option<Utf8PathBuf>,
    text: Option<String>,
    tool: Option<String>,
    args: Option<serde_json::Value>,
    failed: bool,
    result: Option<String>,
    reason: Option<String>,
}

pub(crate) fn parse_wire(stdin: &str) -> Result<CaptureEvent, ParseSkip> {
    let wire: WireCapture =
        serde_json::from_str(stdin).map_err(|_| ParseSkip("invalid payload"))?;
    let native_session_id = wire.session.ok_or(ParseSkip("no session id"))?;
    let kind = match wire.event.as_deref() {
        Some("started") => CaptureKind::Started,
        Some("prompted") => CaptureKind::Prompted {
            text: wire.text.ok_or(ParseSkip("no prompt"))?,
        },
        Some("tool") => CaptureKind::Tool {
            name: wire.tool.ok_or(ParseSkip("no tool name"))?,
            input: wire.args.unwrap_or_default(),
            failed: wire.failed,
            result: wire.result,
        },
        Some("ended") => CaptureKind::Ended {
            reason: wire.reason,
        },
        _ => return Err(ParseSkip("unsupported event")),
    };
    Ok(CaptureEvent {
        kind,
        native_session_id,
        cwd: wire.cwd,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/providers/capture.rs"]
mod tests;
