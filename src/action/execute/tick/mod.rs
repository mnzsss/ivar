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
//! [`ExecutionStatus::Running`] once at least one workstream is launched, and
//! settles again once the wave is over — see "Settling the wave" below.
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
//! # Settling the wave
//!
//! `tick` launches one wave and blocks until every workstream in it is
//! terminal, so by the time the fold loop ends the board is no longer
//! running anything — and a board left at [`ExecutionStatus::Running`] can
//! never be ticked again, because `tick` only accepts
//! [`ExecutionStatus::Approved`]. That stranded every wave after the first:
//! `tick` refused (not approved), `approve` refused (not awaiting approval)
//! and `reply` refused (not blocked), leaving no command able to move the
//! board. So the last thing `tick` does is settle it:
//!
//! - a board that a workstream took to [`ExecutionStatus::Blocked`] stays
//!   blocked — it needs `reply`, not another tick;
//! - a board whose every workstream is [`WorkstreamStatus::Done`] becomes
//!   [`ExecutionStatus::Completed`];
//! - otherwise work remains — workstreams still `Waiting`, typically on the
//!   dependencies this wave just satisfied — and the board returns to
//!   `Approved`, which is where the next `tick` launches the next wave from.
//!   The approval gate is not reopened: the human approved *this* graph and
//!   nothing about it changed.
//!
//! The rule itself lives on [`ExecutionBoard::settle`], because `reply` and
//! `ack-revision` end in exactly the same place — workstreams moved, board
//! status now stale — and three commands deriving the same summary three
//! ways is how the board got stuck in the first place.
//!
//! # Terminal status
//!
//! [`WorkstreamStatus`] has no `Failed` variant — only `Waiting`, `Active`,
//! `Done`, `Blocked`, `Paused`. A clean exit maps to `Done`; a non-zero exit,
//! a signal death, a spawn failure, or an `AskUserQuestion` tool call (Claude
//! Code only — see "A harness that cannot ask") all map to `Blocked` (with
//! `board.blocked_by` naming the workstream and the board following it to
//! [`ExecutionStatus::Blocked`]) — the closest existing
//! status to "stopped, needs a human", mirroring exactly what `reply`
//! reverses. Either way the workstream leaves `Active` for good: it can never
//! stay active after its process is gone, and a later `tick` can make
//! progress on the rest of the graph. Adding a dedicated
//! `WorkstreamStatus::Failed` is outside this module's write contract
//! (`domain::feature.rs`); flagged rather than done silently.
//!
//! # A harness that cannot ask
//!
//! Not every harness can ask a question — OpenCode cannot, headlessly, for the
//! reasons [`crate::harness::stream`] sets out. A workstream on one can only
//! finish or fail; it never reaches `Blocked` waiting for `reply`. That is
//! declared rather than discovered: this module reads
//! [`crate::harness::Capabilities::supports_questions`] and writes one
//! `harness.no_questions` journal entry per such launch, before any child
//! spawns, so a run that never pauses for a human reads as intended behaviour
//! rather than as a question that went missing.
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

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{
    ExecutionBoard, ExecutionStatus, Feature, JournalEntry, WorkstreamStatus, WriteContract,
};
use crate::domain::name::{FeatureName, SessionId};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git;
use crate::harness::Harness;
use crate::infra::{fs, hash};
use crate::store::feature;

use super::super::{discover_hall, read_manifest};
use super::inbox;
use super::prompt;
use crate::action::Ctx;
use crate::action::repo::pull;

mod events;
mod launch;

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
    let mut cannot_ask = Vec::new();
    // Every contract in the wave, unioned once: what the post-run audit
    // measures the worktrees against. The wave shares its worktrees, so a
    // sibling's legitimate write is indistinguishable from this workstream's
    // stray one — see `launch::audit_write_contract`'s "Why the wave's
    // contract, not this workstream's".
    let wave_contract: Vec<String> = board
        .graph
        .workstreams
        .iter()
        .filter(|ws| to_launch.contains(&ws.id))
        .flat_map(|ws| ws.write_contract.iter().cloned())
        .collect();
    for ws in &board.graph.workstreams {
        if !to_launch.contains(&ws.id) {
            continue;
        }
        let provider = ws
            .provider
            .unwrap_or_else(|| manifest.providers().default_provider());
        let harness = Harness::for_provider(provider)?;
        if !harness.capabilities().supports_questions {
            cannot_ask.push((ws.id.clone(), harness.binary()));
        }
        // Answers a human already gave this workstream, if it blocked on a
        // question before: the relaunch is a fresh child, and a prompt
        // without them is the same prompt that produced the question.
        let replies = inbox::read(&layout, &feature, &ws.id)?;
        let prompt_text = prompt::render(&plan_text, ws, &replies)?;
        let session_id = SessionId::new(uuid::Uuid::new_v4().to_string())?;
        let view_dir = layout.feature_session(&feature, &session_id);
        let command = launch::build_spawn_command(
            harness,
            &prompt_text,
            ws,
            &view_dir,
            &layout,
            &feature,
            &session_id,
        );
        command_displays.insert(session_id.to_string(), command.display());
        jobs.push(launch::LaunchJob {
            workstream_id: ws.id.clone(),
            session_id,
            provider,
            view_dir,
            command,
            wave_contract: WriteContract::new(wave_contract.clone()),
        });
    }

    // See "A harness that cannot ask": journalled before any child spawns.
    for (workstream_id, binary) in cannot_ask {
        board.push_journal(JournalEntry::new(
            workstream_id,
            "harness.no_questions",
            format!(
                "`{binary}` cannot ask a question in headless execute mode; this workstream will finish or fail, never block for `ivar feature execute reply`"
            ),
        ));
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
    let (tx, rx) = mpsc::channel::<events::TickEvent>();
    let mut handles = Vec::with_capacity(jobs.len());
    for job in jobs {
        let tx = tx.clone();
        let layout = layout.clone();
        let manifest = manifest.clone();
        let feature_record = feature_record.clone();
        let feature = feature.clone();
        let hall_root = layout.root().to_path_buf();
        handles.push(thread::spawn(move || {
            launch::run_launch(
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
        events::apply_event(&mut board, &layout, &feature, &command_displays, event)?;
    }
    for handle in handles {
        let _ = handle.join();
    }

    // Every workstream this tick launched is terminal now — see "Settling
    // the wave".
    settle(&mut board);

    // Final flush: catches whatever `tool.used` entries accumulated since the
    // last state-transition flush, and the settled status. Every transition
    // already forced its own flush, so this can only add trailing activity,
    // never change an answer already on disk.
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

/// Settle the board once the wave is over and journal what that decided.
/// [`ExecutionBoard::settle`] owns the rule (see the module doc's "Settling
/// the wave"); this adds the entry that makes the transition legible in the
/// journal, and only when the status actually moved — a board still `Blocked`
/// on the workstream that blocked it says nothing new.
fn settle(board: &mut ExecutionBoard) {
    let before = board.status;
    board.settle();
    if board.status == before {
        return;
    }
    let (kind, message) = match board.status {
        ExecutionStatus::Completed => ("board.completed", "Every workstream is done".to_owned()),
        ExecutionStatus::Approved => (
            "wave.completed",
            "Wave finished; board back to `approved` — tick again to launch what is now ready"
                .to_owned(),
        ),
        other => (
            "board.settled",
            format!("Wave finished; board is `{other}`"),
        ),
    };
    board.push_journal(JournalEntry::new("board", kind, message));
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
#[path = "../../../../tests/unit/action/execute/tick.rs"]
mod tests;
