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
#[path = "../../../tests/unit/action/execute/tick.rs"]
mod tests;
