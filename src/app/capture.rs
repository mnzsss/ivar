use std::io::{self, Read as _};

use crate::action::memory::capture::CaptureEnv;
use crate::domain::provider::Provider;

/// A payload past the cap is cut mid-document, fails to parse and is skipped,
/// so a huge tool input can never stall the agent's hook.
const MAX_CAPTURE_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn capture_event(provider: Option<&str>) {
    if let Some(provider) = provider.and_then(|id| id.parse::<Provider>().ok()) {
        crate::action::memory::capture::capture(
            provider,
            &read_capture_payload(),
            &CaptureEnv::from_process(),
        );
    }
}

pub(super) fn is_capture_invocation(args: &[std::ffi::OsString]) -> bool {
    matches!(args, [_, memory, capture, ..] if memory == "memory" && capture == "capture")
}

pub(super) fn lenient_provider(args: &[std::ffi::OsString]) -> Option<&str> {
    let mut args = args.iter().filter_map(|arg| arg.to_str());
    while let Some(arg) = args.next() {
        if arg == "--provider" {
            return args.next();
        }
        if let Some(value) = arg.strip_prefix("--provider=") {
            return Some(value);
        }
    }
    None
}

fn read_capture_payload() -> String {
    let mut bytes = Vec::new();
    let _ = io::stdin()
        .take(MAX_CAPTURE_PAYLOAD_BYTES)
        .read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}
