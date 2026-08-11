//! The per-workstream launch half of `tick`: everything a worker thread
//! needs to materialise its own session, spawn the provider, and drain the
//! child's stream. Never touches the board — see `mod.rs`'s "Who spawns, who
//! owns the board".

use std::sync::mpsc;

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::{Feature, WorkstreamDef};
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::{SessionState, rfc3339_now};
use crate::error::Failure;
use crate::harness::stream::{ExecutorEvent, parse_claude_line, parse_opencode_line};
use crate::harness::{Harness, guard};
use crate::infra::proc;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::super::session::start;
use super::events::TickEvent;

#[cfg(test)]
use super::TEST_STUB_BIN_DIR;

/// Everything computed for one workstream's launch before any worker thread
/// starts — deciding is the calling thread's job; a worker only does I/O.
pub(super) struct LaunchJob {
    pub(super) workstream_id: String,
    pub(super) session_id: SessionId,
    pub(super) provider: Provider,
    pub(super) view_dir: Utf8PathBuf,
    pub(super) command: proc::Command,
}

/// Build the invocation for `harness`'s headless execute mode, with the
/// working directory and the ivar session environment baked in.
///
/// The child's environment carries exactly these five `IVAR_*` variables and
/// whatever it inherits ambiently — never `GIT_AUTHOR_NAME`,
/// `GIT_AUTHOR_EMAIL`, `GIT_COMMITTER_NAME` or `GIT_COMMITTER_EMAIL`
/// A launched executor that inherited an overridden git
/// identity is exactly the failure that produced 16 mis-attributed commits on
/// another branch of this repo — the entire point of letting an agent commit
/// is that it commits as the user, so this function adds nothing that could
/// override that.
pub(super) fn build_spawn_command(
    harness: Harness,
    prompt: &str,
    ws: &WorkstreamDef,
    view_dir: &Utf8Path,
    layout: &Layout,
    feature: &FeatureName,
    session_id: &SessionId,
) -> proc::Command {
    let command = harness
        .execute_command(prompt, ws.model.as_deref(), ws.agent.as_deref())
        .cwd(view_dir.to_path_buf())
        .env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_FEATURE", feature.as_str())
        .env("IVAR_SECRETS_DIR", layout.secrets_dir().as_str())
        .env("IVAR_SESSION_ID", session_id.as_str())
        .env("IVAR_SESSION_PATH", view_dir.as_str());

    #[cfg(test)]
    let command = apply_test_path_stub(command);

    command
}

/// See the `TEST_STUB_BIN_DIR` doc comment: never a real `claude`/`opencode`
/// in a test. A no-op when no test has installed a stub.
#[cfg(test)]
fn apply_test_path_stub(command: proc::Command) -> proc::Command {
    let Some(dir) = TEST_STUB_BIN_DIR.with(|cell| cell.borrow().clone()) else {
        return command;
    };
    let ambient = std::env::var("PATH").unwrap_or_default();
    command.env("PATH", format!("{dir}:{ambient}"))
}

/// Runs entirely on its own thread, owning exactly one child and its parser.
/// Materialises this workstream's own session — view dir, session record,
/// execution guard — spawns its provider, drains its stream, and reports
/// every step as an [`ExecutorEvent`] over `tx`. Never touches the board: see
/// the module doc's "Who spawns, who owns the board" section.
pub(super) fn run_launch(
    layout: Layout,
    manifest: Manifest,
    feature_record: Feature,
    feature: FeatureName,
    hall_root: Utf8PathBuf,
    job: LaunchJob,
    tx: &mpsc::Sender<TickEvent>,
) {
    let send = |event: ExecutorEvent| {
        let _ = tx.send(TickEvent {
            workstream_id: job.workstream_id.clone(),
            session_id: job.session_id.to_string(),
            event,
        });
    };

    if let Err(failure) =
        start::materialise_view_dir(&layout, &manifest, Some(&feature_record), &job.view_dir)
    {
        send(ExecutorEvent::Failed {
            error: failure.to_string(),
        });
        return;
    }

    let started_at = rfc3339_now();
    let mut state = SessionState::new(job.provider, &started_at);
    state.bind(feature.clone(), &started_at);
    if let Err(failure) = state.write(&job.view_dir) {
        send(ExecutorEvent::Failed {
            error: failure.to_string(),
        });
        return;
    }

    if let Err(failure) = guard::materialise(
        job.provider,
        &job.view_dir,
        &hall_root,
        &feature,
        &job.session_id,
    ) {
        send(ExecutorEvent::Failed {
            error: failure.to_string(),
        });
        return;
    }

    let mut child = match proc::stream(&job.command) {
        Ok(child) => child,
        Err(error) => {
            let failure: Failure = error.into();
            send(ExecutorEvent::Failed {
                error: failure.to_string(),
            });
            return;
        }
    };

    send(ExecutorEvent::Started);

    let parse_line: fn(&str) -> Vec<ExecutorEvent> = match job.provider {
        Provider::ClaudeCode => parse_claude_line,
        Provider::OpenCode => parse_opencode_line,
    };

    // OpenCode stamps `sessionID` on every JSON line, so its parser emits a
    // `NativeSession` per line by design (see `harness::stream`'s "The native
    // session id"). The id is announced once here instead — one journal entry,
    // not one per line — and Claude Code, which announces it once anyway, is
    // unaffected.
    let mut native_session_announced = false;
    while let Ok(Some(line)) = child.read_line() {
        for event in parse_line(&line) {
            if matches!(event, ExecutorEvent::NativeSession { .. }) {
                if native_session_announced {
                    continue;
                }
                native_session_announced = true;
            }
            send(event);
        }
    }

    match child.wait() {
        Ok(Some(0)) => send(ExecutorEvent::Completed),
        Ok(Some(code)) => {
            let stderr = child.stderr();
            let error = if stderr.is_empty() {
                format!("exited {code}")
            } else {
                format!("exited {code}: {stderr}")
            };
            send(ExecutorEvent::Failed { error });
        }
        Ok(None) => send(ExecutorEvent::Failed {
            error: "killed by a signal".to_owned(),
        }),
        Err(error) => {
            let failure: Failure = error.into();
            send(ExecutorEvent::Failed {
                error: failure.to_string(),
            });
        }
    }
}
