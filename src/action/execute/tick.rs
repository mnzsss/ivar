//! `ivar feature execute tick` — find ready workstreams and launch them, for
//! real.
//!
//! # What it does
//!
//! Reads the feature's [`ExecutionBoard`], finds workstreams whose declared
//! [`WorkstreamDef::depends_on`] are all [`WorkstreamStatus::Done`], and for
//! each:
//!
//! 1. Validates the plan fingerprint against the board; if diverged, marks the
//!    workstream [`WorkstreamStatus::Blocked`] and does NOT launch.
//! 2. Renders its prompt from the plan (see [`super::prompt`]), materialises a
//!    real session — view dir, session record, execution guard — and spawns
//!    the resolved [`Harness`]'s headless execute command via
//!    [`crate::infra::proc::stream`], recording the session id in
//!    [`ExecutionBoard::sessions`] so `guard-check --session <id>` resolves it
//!    instead of answering "unknown session".
//! 3. Transitions the workstream from [`WorkstreamStatus::Waiting`] to
//!    [`WorkstreamStatus::Active`].
//!
//! The board's overall status advances from [`ExecutionStatus::Approved`] to
//! [`ExecutionStatus::Running`] once at least one workstream is launched.
//!
//! `tick` **blocks** until every workstream it launched has reached a
//! terminal state — `Done` on a clean exit, `Blocked` on a failure or a
//! question (see "Terminal status" below) — because a real child process has
//! a real exit code to wait for. Tick with nothing ready is a no-op that
//! reports so and never spawns anything.
//!
//! # Who spawns, who owns the board
//!
//! One `std::thread` per ready workstream owns exactly one child and its
//! stdout parser. It never touches the board: it materialises its own session
//! (view dir, session record, guard), spawns, drains lines through
//! [`crate::harness::stream::parse_claude_line`] /
//! [`parse_opencode_line`](crate::harness::stream::parse_opencode_line), and
//! sends every [`ExecutorEvent`] it produces — plus `Started` on a successful
//! spawn and `Completed`/`Failed` from the child's own exit — over an
//! `mpsc::channel`. The *calling* thread is the sole owner of the board: it
//! assigns `next_event_seq`, appends journal entries, and writes. This was
//! chosen over `Arc<Mutex<ExecutionBoard>>` deliberately — see the analysis's
//! "Who owns the board" — because most events are `tool.used` noise a lock
//! would make every worker serialise on, and every write rewrites the whole
//! file. No lock, no contention, and the sequence stays monotonic because
//! only one thread ever advances it.
//!
//! Every child is spawned before any stream is drained: sessions are
//! registered and workstreams flip to `Active` synchronously, in the calling
//! thread, *before* any worker thread starts (so the guard resolves the
//! instant the fastest child's first tool call arrives), and then every
//! worker runs on its own OS thread rather than a sequential loop — the
//! mechanism that keeps the slowest startup from queueing behind its
//! siblings' drain loops.
//!
//! # Write cadence
//!
//! A state transition (`started`, `question.asked`, `native_session`,
//! `session.completed`, `session.failed`) forces an immediate flush, so the
//! board is never wrong about status on disk. `tool.used` only appends to the
//! in-memory journal; the final flush after every worker has joined catches
//! up on whatever accumulated, so a crash mid-tick can only lose trailing
//! activity noise, never a status.
//!
//! # Terminal status
//!
//! [`WorkstreamStatus`] has no `Failed` variant — only `Waiting`, `Active`,
//! `Done`, `Blocked`, `Paused`. A clean exit maps to `Done`; a non-zero exit,
//! a signal death, a spawn failure, or an `AskUserQuestion` tool call all map
//! to `Blocked` (with `board.blocked_by` naming the workstream and the board
//! following it to [`ExecutionStatus::Blocked`]) — the closest existing
//! status to "stopped, needs a human", mirroring exactly what `reply`
//! reverses. Either way the workstream leaves `Active` for good: it can never
//! stay active after its process is gone, and a later `tick` can make
//! progress on the rest of the graph. Adding a dedicated
//! `WorkstreamStatus::Failed` is outside this module's write contract
//! (`domain::feature.rs`); flagged rather than done silently.
//!
//! # The native session id
//!
//! [`ExecutorEvent::NativeSession`] — the id `--resume` accepts — is persisted
//! as its own journal entry (`kind: "native_session"`), because
//! [`ExecutionBoard`] has no dedicated field for it and adding one is, again,
//! outside this module's write contract. A future `reply` can read it back
//! off the journal.
//!
//! # Never a real provider in a test
//!
//! Every test that exercises a launch stubs `claude` on `PATH` — see the
//! `TEST_STUB_BIN_DIR` doc comment below for why that stub is a thread-local
//! rather than a mutation of the process's own `PATH`.

use std::collections::BTreeMap;
use std::io;
use std::sync::mpsc;
use std::thread;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{
    ExecutionBoard, ExecutionStatus, Feature, JournalEntry, WorkstreamDef, WorkstreamStatus,
};
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::{SessionState, rfc3339_now};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git;
use crate::harness::stream::{ExecutorEvent, parse_claude_line, parse_opencode_line};
use crate::harness::{Harness, guard};
use crate::infra::{fs, hash, proc};
use crate::store::feature;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::prompt;
use crate::action::Ctx;
use crate::action::repo::pull;
use crate::action::session::start;

/// What `ivar feature execute tick` needs.
#[derive(Debug, Clone)]
pub struct TickInput {
    /// The feature whose board to tick — find ready workstreams and launch them.
    pub feature: String,
}

/// What `ivar feature execute tick` did.
#[derive(Debug, Clone, Serialize)]
pub struct TickOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the board belongs to.
    pub feature: FeatureName,
    /// Workstreams that were launched, in order.
    pub launched: Vec<String>,
    /// The board after the tick.
    pub board: ExecutionBoard,
    /// Where the board was written.
    pub board_path: Utf8PathBuf,
}

impl WriteHuman for TickOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.launched.is_empty() {
            writeln!(
                w,
                "Tick: nothing ready for `{}` — no workstreams to launch",
                self.feature
            )?;
        } else {
            writeln!(
                w,
                "Tick: launched {} for `{}` at {}",
                if self.launched.len() == 1 {
                    "workstream"
                } else {
                    "workstreams"
                },
                self.feature,
                self.board_path
            )?;
            for ws in &self.launched {
                writeln!(w, "  - {ws}")?;
            }
        }
        Ok(())
    }
}

/// Find ready workstreams on `input.feature`'s board and launch them.
///
/// Blocked when the feature has no board or the board is not in
/// `Approved` status — only an approved board may be ticked. A divergent
/// plan fingerprint blocks individual workstreams rather than launching them.
/// Tick with nothing ready is a no-op.
pub fn tick(ctx: &Ctx, input: TickInput) -> Outcome<TickOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;

    let mut board = match ExecutionBoard::read(&layout, &feature)? {
        Some(b) => b,
        None => {
            return Err(Failure::blocked(
                "execute.board_missing",
                format!("no execution board for feature `{feature}`"),
            )
            .expected("a prepared execution board")
            .actual("board.json does not exist under the feature's execution directory")
            .fix(FixAction::safe(
                "execute.prepare_first",
                format!(
                    "Prepare the board first: `ivar feature execute prepare {feature} --graph-json <path>`."
                ),
            )))
        }
    };

    if board.status != ExecutionStatus::Approved {
        return Err(Failure::blocked(
            "execute.board_not_approved",
            format!(
                "cannot tick board for `{feature}`: board is `{}`, expected `approved`",
                board.status
            ),
        )
        .expected("an approved board")
        .actual(format!("board status is `{}`", board.status))
        .fix(FixAction::safe(
            "execute.approve_first",
            format!("Approve the board first: `ivar feature execute approve {feature}`."),
        )));
    }

    // Validate plan fingerprint against the board. If diverged, block ALL
    // waiting workstreams and do NOT launch any.
    let board_fingerprint = board.graph.plan_fingerprint.clone();
    let plan_path = layout.plan_dir(&feature).join("plan.md");
    let current_fingerprint = hash::file(&plan_path).ok();

    if let Some(fp) = &current_fingerprint
        && fp != &board_fingerprint
    {
        // Divergent plan — block all waiting workstreams.
        for ws in &mut board.graph.workstreams {
            if ws.status == WorkstreamStatus::Waiting {
                ws.status = WorkstreamStatus::Blocked;
            }
        }
        board.push_journal(JournalEntry::new(
            "board",
            "diverged",
            format!(
                "Plan diverged from board fingerprint (expected {}, got {})",
                board_fingerprint, fp
            ),
        ));
        board.write(&layout, &feature)?;

        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(TickOutcome {
            root: layout.root().to_path_buf(),
            feature,
            launched: Vec::new(),
            board,
            board_path,
        }));
    }

    // Which workstreams are ready: `Waiting` with every dependency `Done`.
    let to_launch: Vec<String> = board
        .graph
        .workstreams
        .iter()
        .filter(|ws| ws.status == WorkstreamStatus::Waiting)
        .filter(|ws| {
            ws.depends_on.iter().all(|dep_id| {
                board
                    .graph
                    .workstreams
                    .iter()
                    .any(|w| w.id == *dep_id && w.status == WorkstreamStatus::Done)
            })
        })
        .map(|ws| ws.id.clone())
        .collect();

    if to_launch.is_empty() {
        board.write(&layout, &feature)?;
        let board_path = feature::board_path(&layout, &feature);
        return Ok(Report::new(TickOutcome {
            root: layout.root().to_path_buf(),
            feature,
            launched: Vec::new(),
            board,
            board_path,
        }));
    }

    let manifest = read_manifest(&layout)?;
    let feature_record =
        Feature::read(&layout, &feature)?.ok_or_else(|| feature_vanished(&feature))?;
    let plan_text = fs::read_text(&plan_path)?.ok_or_else(|| plan_vanished(&feature))?;

    // The network refresh runs once per tick, before the
    // fan-out, and only when something is actually ready to launch — never
    // once per workstream. Mirrors `session::start`'s Smart Fetch: best-effort
    // per repo, a failure warns rather than blocking the tick.
    let mut warnings = Vec::new();
    for repo in manifest.repos() {
        match pull::refresh_default(&git::System, &layout, repo) {
            pull::PullStatus::Refreshed => {}
            pull::PullStatus::Failed { reason } => warnings.push(Warning::new(
                "execute.tick_smart_fetch_failed",
                repo.name().to_string(),
                reason,
            )),
            pull::PullStatus::Skipped { reason } => warnings.push(Warning::new(
                "execute.tick_smart_fetch_skipped",
                repo.name().to_string(),
                reason,
            )),
        }
    }

    // Build every launch's command up front. Pure computation over data
    // already in hand (the plan text, the workstream) — no I/O — so a
    // workstream claiming an operation the plan does not back refuses the
    // whole tick before anything spawns, rather than half the fan-out
    // succeeding and the rest silently never starting.
    let mut jobs = Vec::with_capacity(to_launch.len());
    let mut command_displays = BTreeMap::new();
    for ws in &board.graph.workstreams {
        if !to_launch.contains(&ws.id) {
            continue;
        }
        let provider = ws
            .provider
            .unwrap_or_else(|| manifest.providers().default_provider());
        let harness = Harness::for_provider(provider)?;
        let prompt_text = prompt::render(&plan_text, ws)?;
        let session_id = SessionId::new(uuid::Uuid::new_v4().to_string())?;
        let view_dir = layout.feature_session(&feature, &session_id);
        let command = build_spawn_command(
            harness,
            &prompt_text,
            ws,
            &view_dir,
            &layout,
            &feature,
            &session_id,
        );
        command_displays.insert(session_id.to_string(), command.display());
        jobs.push(LaunchJob {
            workstream_id: ws.id.clone(),
            session_id,
            provider,
            view_dir,
            command,
        });
    }

    // Register every session and flip its workstream to `Active`
    // synchronously, before any child spawns, and flush once: `guard-check
    // --session <id>` has to resolve the session the instant the fastest
    // child's first tool call arrives, not whenever the calling thread
    // happens to fold that workstream's `Started` event off the channel.
    for job in &jobs {
        board
            .sessions
            .insert(job.session_id.to_string(), job.workstream_id.clone());
    }
    for ws in &mut board.graph.workstreams {
        if to_launch.contains(&ws.id) {
            ws.status = WorkstreamStatus::Active;
        }
    }
    if board.status == ExecutionStatus::Approved {
        board.set_status(ExecutionStatus::Running);
    }
    board.write(&layout, &feature)?;

    let launched = to_launch;

    // Fan out: one thread per workstream. Every child is spawned before any
    // stream is drained because each spawn happens on its own OS thread
    // rather than in a shared sequential loop — the mechanism, not merely the
    // intent, behind "the slowest startup never queues behind its siblings".
    let (tx, rx) = mpsc::channel::<TickEvent>();
    let mut handles = Vec::with_capacity(jobs.len());
    for job in jobs {
        let tx = tx.clone();
        let layout = layout.clone();
        let manifest = manifest.clone();
        let feature_record = feature_record.clone();
        let feature = feature.clone();
        let hall_root = layout.root().to_path_buf();
        handles.push(thread::spawn(move || {
            run_launch(
                layout,
                manifest,
                feature_record,
                feature,
                hall_root,
                job,
                &tx,
            );
        }));
    }
    drop(tx);

    // Fold every event as it arrives. The channel closes once every
    // worker's own `Sender` clone is dropped — i.e. once every worker has
    // finished — so this loop is also what makes `tick` block until every
    // launched workstream reaches a terminal state.
    for event in rx {
        apply_event(&mut board, &layout, &feature, &command_displays, event)?;
    }
    for handle in handles {
        let _ = handle.join();
    }

    // Final flush: catches whatever `tool.used` entries accumulated since the
    // last state-transition flush. Every transition already forced its own
    // flush, so this can only add trailing activity, never change an answer
    // already on disk.
    board.write(&layout, &feature)?;

    let board_path = feature::board_path(&layout, &feature);
    Ok(Report::with_warnings(
        TickOutcome {
            root: layout.root().to_path_buf(),
            feature,
            launched,
            board,
            board_path,
        },
        warnings,
    ))
}

/// A feature with an approved board but no `feature.json` — a race between
/// this read and whatever deleted it, not a normal outcome.
fn feature_vanished(feature: &FeatureName) -> Failure {
    Failure::blocked(
        "execute.tick_feature_vanished",
        format!("feature `{feature}` has an approved board but no feature.json"),
    )
    .expected("the feature record this board belongs to")
    .actual("feature.json does not exist")
    .fix(FixAction::safe(
        "execute.check_feature",
        format!("Check that `{feature}` still exists: `ivar feature status {feature}`."),
    ))
}

/// A feature with an approved board but no `plan.md` to render a prompt
/// from — the prompt renderer needs the plan's text, not just its
/// fingerprint.
fn plan_vanished(feature: &FeatureName) -> Failure {
    Failure::blocked(
        "execute.tick_plan_missing",
        format!("feature `{feature}` has an approved board but no plan.md to render prompts from"),
    )
    .expected("plan.md under the feature's plan directory")
    .actual("plan.md does not exist")
    .fix(FixAction::safe(
        "execute.restore_plan",
        "Restore plan.md, or re-run `ivar feature plan` for this feature.",
    ))
}

/// Everything computed for one workstream's launch before any worker thread
/// starts — deciding is the calling thread's job; a worker only does I/O.
struct LaunchJob {
    workstream_id: String,
    session_id: SessionId,
    provider: Provider,
    view_dir: Utf8PathBuf,
    command: proc::Command,
}

/// One [`ExecutorEvent`] from one worker, tagged with which workstream and
/// session it belongs to — the vocabulary the calling thread folds into the
/// board.
struct TickEvent {
    workstream_id: String,
    session_id: String,
    event: ExecutorEvent,
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
fn build_spawn_command(
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
fn run_launch(
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

    while let Ok(Some(line)) = child.read_line() {
        for event in parse_line(&line) {
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

/// Fold one worker's [`TickEvent`] into the board. The calling thread is the
/// sole owner of the board (see the module doc); this is the only function
/// that mutates it once launches begin. A state transition forces an
/// immediate flush; `ToolUsed` only appends to the in-memory journal — see
/// the module doc's "Write cadence".
fn apply_event(
    board: &mut ExecutionBoard,
    layout: &Layout,
    feature: &FeatureName,
    command_displays: &BTreeMap<String, String>,
    tick_event: TickEvent,
) -> Result<(), Failure> {
    let TickEvent {
        workstream_id,
        session_id,
        event,
    } = tick_event;
    board.next_event_seq += 1;
    let seq = board.next_event_seq;

    let mut flush = true;
    match event {
        ExecutorEvent::Started => {
            let command_display = command_displays
                .get(&session_id)
                .map(String::as_str)
                .unwrap_or_default();
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("started.{session_id}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: "started".to_owned(),
                message: format!("Launched session {session_id} ({command_display})"),
            });
        }
        ExecutorEvent::ToolUsed { tool, path } => {
            let message = match path {
                Some(path) => format!("{tool} {path}"),
                None => tool,
            };
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("tool.used.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: "tool.used".to_owned(),
                message,
            });
            flush = false;
        }
        ExecutorEvent::QuestionAsked { prompt: question } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("question.asked.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "question.asked".to_owned(),
                message: question,
            });
            block_on(board, &workstream_id);
        }
        ExecutorEvent::NativeSession { id } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("native_session.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id,
                kind: "native_session".to_owned(),
                message: format!("Native session id: {id}"),
            });
        }
        ExecutorEvent::Completed => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("session.completed.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "session.completed".to_owned(),
                message: format!("Session {session_id} completed"),
            });
            for ws in &mut board.graph.workstreams {
                if ws.id == workstream_id {
                    ws.status = WorkstreamStatus::Done;
                }
            }
        }
        ExecutorEvent::Failed { error } => {
            board.push_journal(JournalEntry {
                seq,
                event_id: format!("session.failed.{session_id}.{seq}"),
                timestamp: now_epoch_seconds(),
                workstream: workstream_id.clone(),
                kind: "session.failed".to_owned(),
                message: error,
            });
            block_on(board, &workstream_id);
        }
    }

    if flush {
        board.write(layout, feature)?;
    }
    Ok(())
}

/// A workstream that cannot proceed on its own — it asked a question, or its
/// process failed — moves to [`WorkstreamStatus::Blocked`], the terminal-ish
/// status closest to "stopped, needs a human" the domain model has (there is
/// no `WorkstreamStatus::Failed` — see the module doc's "Terminal status"
/// section). The board follows it to [`ExecutionStatus::Blocked`], the exact
/// transition `reply` reverses.
fn block_on(board: &mut ExecutionBoard, workstream_id: &str) {
    for ws in &mut board.graph.workstreams {
        if ws.id == workstream_id {
            ws.status = WorkstreamStatus::Blocked;
        }
    }
    board.blocked_by = Some(workstream_id.to_owned());
    board.set_status(ExecutionStatus::Blocked);
}

fn now_epoch_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}

// -- Test-only PATH stubbing --------------------------------------------
//
// A test must never spawn the real `claude`/`opencode` a developer's machine
// happens to have on `PATH` — doing so during `cargo test` would shell out to
// a live agent binary. The obvious fix, prepending a stub directory to the
// process's own `PATH` for the duration of a test, needs `std::env::set_var`,
// which has required an `unsafe` block since it stopped being sound to call
// concurrently — and this crate's `[lints.rust] unsafe_code = "forbid"` makes
// that a hard compile error, everywhere, including here.
//
// So: no process-wide mutation. Instead, [`build_spawn_command`]
// reads this thread-local when compiled for tests, and bakes
// a `PATH` override — stub directory first — directly into the
// [`crate::infra::proc::Command`] it builds, entirely on the *calling*
// thread. That command is then moved as a value into the worker thread that
// actually spawns it, so the worker never needs to see the thread-local
// itself. Thread-local rather than a `static Mutex` for the same reason it
// solves the concurrency problem for free: `cargo test` runs tests on
// separate OS threads, and each test's stub must never leak into a sibling
// running at the same time.
#[cfg(test)]
thread_local! {
    static TEST_STUB_BIN_DIR: std::cell::RefCell<Option<Utf8PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::execute::approve::{self as approve_action, ApproveInput};
    use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
    use crate::action::feature::create::{
        self as feature_create, CreateInput as FeatureCreateInput,
    };
    use crate::action::hall::{self, InitInput};
    use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
    use crate::error::Status;
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::test_support::hall_root;

    const GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-a",
                "title": "A",
                "operations": ["op-a"],
                "depends_on": [],
                "write_contract": ["src/a"]
            },
            {
                "id": "ws-b",
                "title": "B",
                "operations": ["op-b"],
                "depends_on": ["ws-a"],
                "write_contract": ["src/b"]
            }
        ]
    }"#;

    /// A plan whose `## Operations` section backs every operation
    /// `GRAPH_JSON` claims — `tick` now renders each workstream's prompt from
    /// this text via [`prompt::render`], which refuses a workstream that
    /// claims an operation the plan does not document.
    const PLAN_TEXT: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **op-a** — Do the first thing.\n\
        \n\
        **op-b** — Do the second thing.\n\
        \n\
        ## Operations\n\
        \n\
        ### ws-a\n\
        - op-a\n\
        write_contract:\n\
        - src/a\n\
        \n\
        ### ws-b\n\
        - op-b\n\
        write_contract:\n\
        - src/b\n";

    /// A hall with a feature, a plan, and a prepared+approved board.
    fn approved_board() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();
        feature_create::create(
            &ctx,
            FeatureCreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();
        plan_create::create(
            &ctx,
            PlanCreateInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        // Overwrite the scaffolded plan.md with one that documents op-a/op-b
        // under each workstream's own heading, before `prepare` fingerprints
        // it — the fingerprint prepare records has to match what is on disk
        // by the time `tick` reads it back, or the divergence check fires.
        fs::write_text(&root.join("plans/checkout/plan.md"), PLAN_TEXT).unwrap();
        let graph = root.join("graph.json");
        fs::write_text(&graph, GRAPH_JSON).unwrap();
        prepare_action::prepare(
            &ctx,
            PrepareInput {
                feature: "checkout".to_owned(),
                graph_json: graph.to_string(),
            },
        )
        .unwrap();
        approve_action::approve(
            &ctx,
            ApproveInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        (guard, root)
    }

    /// The board read back off disk.
    fn persisted(root: &Utf8PathBuf) -> ExecutionBoard {
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        ExecutionBoard::read(&layout, &feature).unwrap().unwrap()
    }

    #[test]
    fn tick_launches_workstreams_with_met_dependencies() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Mark ws-a as Done so ws-b becomes ready.
        {
            let layout = Layout::at(root.clone());
            let feature = FeatureName::new("checkout").unwrap();
            let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
            for ws in &mut board.graph.workstreams {
                if ws.id == "ws-a" {
                    ws.status = WorkstreamStatus::Done;
                }
            }
            board.write(&layout, &feature).unwrap();
        }

        let _stub = PathStub::install("claude", "exit 0");
        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.launched, vec!["ws-b"]);
        assert_eq!(persisted(&root).status, ExecutionStatus::Running);

        // Session→workstream link recorded.
        let on_disk = persisted(&root);
        assert_eq!(on_disk.sessions.len(), 1);
        let (sess_id, ws_id) = on_disk.sessions.iter().next().unwrap();
        assert_eq!(ws_id, "ws-b");
        assert!(!sess_id.is_empty());
    }

    #[test]
    fn tick_blocks_when_plan_diverges() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Tamper with plan.md behind ivar's back — changes the fingerprint.
        fs::write_text(
            &root.join("plans/checkout/plan.md"),
            "# Plan\n\n- changed content\n",
        )
        .unwrap();

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert!(
            report.value.launched.is_empty(),
            "divergent plan must block"
        );

        // Workstream marked as Blocked.
        let on_disk = persisted(&root);
        assert_eq!(
            on_disk.graph.workstreams[0].status,
            WorkstreamStatus::Blocked
        );

        // Journal records the divergence.
        let last_entry = on_disk.journal.last().unwrap();
        assert_eq!(last_entry.kind, "diverged");
    }

    #[test]
    fn tick_with_nothing_ready_is_a_no_op() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // No workstream is Done, so ws-b's dependency is unmet.
        // ws-a has no dependencies but... actually ws-a HAS no deps and IS
        // Waiting, so it should launch. Let me check: depends_on is empty
        // for ws-a, so deps_met is true, meaning ws-a WILL launch.
        // To test "nothing ready", I need to set up a scenario where no
        // workstream can launch. Let me modify the setup.

        // Actually, with the current graph, ws-a has no deps and will always
        // launch. Let me create a board where ws-a depends on itself (circular)
        // or just accept that ws-a launches and test differently.

        // For "nothing ready" test, let me manually set up a board where
        // ws-a depends on ws-b which depends on ws-a (circular deps).
        let layout = Layout::at(root.clone());
        let feature = FeatureName::new("checkout").unwrap();
        let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();

        // Make both depend on each other — circular, neither can launch.
        board.graph.workstreams[0].depends_on = vec!["ws-b".to_owned()];
        board.graph.workstreams[1].depends_on = vec!["ws-a".to_owned()];

        board.write(&layout, &feature).unwrap();

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert!(
            report.value.launched.is_empty(),
            "circular deps: nothing should launch"
        );
    }

    #[test]
    fn tick_refuses_a_board_not_in_approved_status() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Set board to Pending.
        {
            let layout = Layout::at(root.clone());
            let feature = FeatureName::new("checkout").unwrap();
            let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
            board.set_status(ExecutionStatus::Pending);
            board.write(&layout, &feature).unwrap();
        }

        let failure = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "execute.board_not_approved");
        assert!(failure.what.contains("pending"));
    }

    #[test]
    fn the_agent_reaches_the_provider_command_line() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        // Give ws-a a custom agent.
        {
            let layout = Layout::at(root.clone());
            let feature = FeatureName::new("checkout").unwrap();
            let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
            if let Some(ws) = board
                .graph
                .workstreams
                .iter_mut()
                .find(|ws| ws.id == "ws-a")
            {
                ws.agent = Some("custom-agent".to_owned());
            }
            board.write(&layout, &feature).unwrap();
        }

        let _stub = PathStub::install("claude", "exit 0");
        let _report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        // Verify via journal entry that the agent appears in the command.
        let on_disk = persisted(&root);
        let started_entries: Vec<_> = on_disk
            .journal
            .iter()
            .filter(|e| e.kind == "started")
            .collect();
        assert!(!started_entries.is_empty(), "must have started entries");
        let msg = &started_entries[0].message;
        assert!(msg.contains("custom-agent"), "agent must reach CLI: {msg}");
    }

    #[test]
    fn the_session_to_workstream_link_is_recorded() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        let _stub = PathStub::install("claude", "exit 0");
        let _report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        let on_disk = persisted(&root);
        assert_eq!(
            on_disk.sessions.len(),
            1,
            "exactly one session launched (ws-a)"
        );
        let (session_id, workstream_id) = on_disk.sessions.iter().next().unwrap();
        assert_eq!(workstream_id, "ws-a");
        // Session ID is a valid UUID.
        assert!(uuid::Uuid::parse_str(session_id).is_ok());
    }

    // --- the child's git identity ------------------------------------------

    /// The child's environment carries the ivar session variables and
    /// nothing this module adds beyond them — never a git identity override.
    /// The stub dumps its own `env` to a file (the only reliable way to
    /// inspect a child's actual environment) rather than the test reading
    /// `/proc` or similar.
    #[test]
    fn the_child_env_carries_ivar_session_vars_but_never_a_git_identity() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        let (_env_guard, env_dir) = crate::test_support::utf8_temp_dir();
        let env_dump = env_dir.join("child-env.txt");
        let _stub = PathStub::install("claude", &format!("env > '{env_dump}'\n"));

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        assert!(report.is_clean());

        let dumped = fs::read_text(&env_dump).unwrap().unwrap();
        assert!(
            dumped.contains("IVAR_SESSION_ID="),
            "ivar session vars must reach the child: {dumped}"
        );
        assert!(dumped.contains("IVAR_FEATURE=checkout"), "was: {dumped}");
        for var in [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
        ] {
            assert!(
                !dumped
                    .lines()
                    .any(|line| line.starts_with(&format!("{var}="))),
                "{var} must never reach the child: {dumped}"
            );
        }
    }

    // --- folding the provider's events into the board -----------------------

    /// A tool call and the provider's native session id, both emitted on the
    /// stub's stdout as real `stream-json` lines, fold into the journal as
    /// `tool.used` and `native_session` — and a clean exit reaches `Done`.
    #[test]
    fn stream_events_fold_into_the_journal_and_a_clean_exit_reaches_done() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        let script = r#"printf '%s\n' '{"type":"system","subtype":"init","session_id":"native-xyz-789"}'
printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"src/a/lib.rs"}}]}}'
exit 0
"#;
        let _stub = PathStub::install("claude", script);

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        assert!(report.is_clean());

        let on_disk = persisted(&root);
        assert!(
            on_disk.journal.iter().any(|e| e.kind == "tool.used"
                && e.message.contains("Read")
                && e.message.contains("src/a/lib.rs")),
            "tool.used must be in the journal: {:?}",
            on_disk.journal
        );
        assert!(
            on_disk
                .journal
                .iter()
                .any(|e| e.kind == "native_session" && e.message.contains("native-xyz-789")),
            "the native session id must be persisted: {:?}",
            on_disk.journal
        );
        assert!(
            on_disk
                .journal
                .iter()
                .any(|e| e.kind == "session.completed"),
            "a clean exit must journal session.completed: {:?}",
            on_disk.journal
        );

        let ws_a = on_disk
            .graph
            .workstreams
            .iter()
            .find(|w| w.id == "ws-a")
            .unwrap();
        assert_eq!(ws_a.status, WorkstreamStatus::Done);
    }

    // --- terminal status ----------------------------------------------------

    /// A process that exits non-zero reaches a terminal status — never left
    /// `Active` — and a later `tick` gets a clear, structured refusal rather
    /// than hanging or silently doing nothing: the board is recoverable via
    /// `reply`, not by deleting and re-preparing it.
    #[test]
    fn a_failed_child_reaches_a_terminal_status_not_active_forever() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        let _stub = PathStub::install("claude", "printf trouble >&2\nexit 3\n");

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
        assert!(report.is_clean());

        let on_disk = persisted(&root);
        let ws_a = on_disk
            .graph
            .workstreams
            .iter()
            .find(|w| w.id == "ws-a")
            .unwrap();
        assert_ne!(
            ws_a.status,
            WorkstreamStatus::Active,
            "a dead process must never leave its workstream Active"
        );
        assert_eq!(ws_a.status, WorkstreamStatus::Blocked);
        assert_eq!(on_disk.status, ExecutionStatus::Blocked);
        assert_eq!(on_disk.blocked_by.as_deref(), Some("ws-a"));
        assert!(
            on_disk.journal.iter().any(|e| e.kind == "session.failed"
                && e.message.contains("exited 3")
                && e.message.contains("trouble")),
            "the failure must be journaled with its exit code and stderr: {:?}",
            on_disk.journal
        );

        // A later tick does not hang and does not need the board deleted: it
        // gets a clear, structured refusal naming the blocked status.
        let second = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap_err();
        assert_eq!(second.code, "execute.board_not_approved");
    }

    // -- OP-TEST-REPRO: prove the bug before fixing it -----------------------
    //
    // A stub `claude` goes on `PATH` — see the module-level comment on
    // `TEST_STUB_BIN_DIR` for why a thread-local rather than mutating the
    // process's real `PATH`. While the guard is alive, anything this test's
    // thread builds a spawn command on sees the stub first.

    /// RAII: while alive, an executable named `name` running `script_body`
    /// (under `#!/bin/sh`) is first on the `PATH` this test's thread builds
    /// spawn commands against. Dropping clears the seam.
    struct PathStub {
        _dir: tempfile::TempDir,
    }

    impl PathStub {
        fn install(name: &str, script_body: &str) -> Self {
            let (dir, bin_dir) = crate::test_support::utf8_temp_dir();
            let script_path = bin_dir.join(name);
            fs::write_text(&script_path, &format!("#!/bin/sh\n{script_body}\n")).unwrap();
            fs::chmod(&script_path, 0o755).unwrap();
            TEST_STUB_BIN_DIR.with(|cell| *cell.borrow_mut() = Some(bin_dir));
            Self { _dir: dir }
        }
    }

    impl Drop for PathStub {
        fn drop(&mut self) {
            TEST_STUB_BIN_DIR.with(|cell| *cell.borrow_mut() = None);
        }
    }

    /// The bug, proven: a stub `claude` on `PATH` touches a sentinel file the
    /// instant it runs. `tick`'s current loop builds a command *string* and
    /// never spawns anything, so the sentinel must never appear — this is an
    /// external effect, not a journal or `board.sessions` assertion, because
    /// the current fake harness already writes those correctly (that is
    /// exactly how it hides the bug). Demonstrated failing against the
    /// unmodified launch loop, before the real spawn replaced it.
    #[test]
    fn tick_actually_spawns_the_provider() {
        let (_guard, root) = approved_board();
        let ctx = Ctx::new(root.clone());

        let (_sentinel_guard, sentinel_dir) = crate::test_support::utf8_temp_dir();
        let sentinel = sentinel_dir.join("claude-ran");
        let _stub = PathStub::install("claude", &format!("touch '{sentinel}'\n"));

        let report = tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();

        assert!(report.is_clean());
        assert!(
            fs::is_file(&sentinel).unwrap(),
            "tick must actually spawn `claude`, not just record a fake session"
        );
    }
}
