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
//!    workstream [`WorkstreamStatus::Blocked`] and does NOT launch. Reported
//!    as a warning (`execute.plan_diverged`) carrying a [`PlanDivergence`],
//!    never as a clean run — otherwise a tick that stopped everything it found
//!    and one that found nothing are the same exit code and the same sentence.
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
//! `Done`, `Blocked`, `Paused`. A clean exit maps to `Done` **only when the
//! workstream has something to show for itself** — see "Done is earned, not
//! inherited" below; a non-zero exit,
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
//! # Done is earned, not inherited
//!
//! A clean exit is the child's claim that it finished, not evidence that it
//! did anything: a session that was denied every write, or misread its prompt,
//! or simply idled, exits zero exactly like one that did the work. `Done` read
//! straight off the exit code therefore asserted work that did not exist — and
//! every workstream depending on it then launched against that assertion.
//!
//! So the post-run audit answers a second question from the same change set it
//! already computes for violations: did anything change under *this*
//! workstream's own contract (see `launch::AuditOutcome`)? When it did, the
//! run is journalled `produced`, naming the paths. A clean exit with no
//! `produced` entry — this run or any earlier one — is journalled
//! `session.unproductive` and blocks instead, because there is no work behind
//! the `done` it was about to claim.
//!
//! Two things keep that from firing on honest runs:
//!
//! - **The relaunch escape.** A workstream that blocked on a question is
//!   relaunched from scratch against a baseline that already holds what its
//!   first run wrote, so its second run legitimately changes nothing new.
//!   The question asked is therefore "has this workstream ever produced",
//!   answered from the append-only journal, not "did this run produce". See
//!   `events::has_ever_produced`.
//! - **No oracle, no refusal.** A feature with no promoted worktree has no
//!   filesystem to read, and "produced nothing" and "nowhere to produce" look
//!   identical from an empty change set. The audit reports which of the two it
//!   saw (`ExecutorEvent::Completed`'s `audited`), and a workstream is never
//!   refused for the absence of an oracle.
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

/// The plan moved out from under the board: `plan.md` no longer hashes to the
/// fingerprint the graph was approved against.
///
/// Carried on the outcome rather than left to the journal: it is the reason
/// nothing launched, and the caller decides what to do next on that reason.
#[derive(Debug, Clone, Serialize)]
pub struct PlanDivergence {
    /// The fingerprint the board was approved against.
    pub approved: String,
    /// What `plan.md` hashes to now.
    pub current: String,
    /// The workstreams this divergence took from `Waiting` to `Blocked`.
    pub blocked: Vec<String>,
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
    /// Set when the tick launched nothing *because* the plan diverged — the
    /// difference between "nothing was ready" and "everything was stopped".
    /// `None` on every ordinary tick.
    pub diverged: Option<PlanDivergence>,
}

impl WriteHuman for TickOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if let Some(divergence) = &self.diverged {
            writeln!(
                w,
                "Tick: plan diverged for `{}` — nothing launched, {} {} blocked",
                self.feature,
                divergence.blocked.len(),
                if divergence.blocked.len() == 1 {
                    "workstream"
                } else {
                    "workstreams"
                }
            )?;
            for ws in &divergence.blocked {
                writeln!(w, "  - {ws}")?;
            }
            writeln!(
                w,
                "plan.md diverged from the graph's fingerprint\n  \
                 approved: {}\n  current:  {}",
                divergence.approved, divergence.current
            )?;
        } else if self.launched.is_empty() {
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
        let mut blocked = Vec::new();
        for ws in &mut board.graph.workstreams {
            if ws.status == WorkstreamStatus::Waiting {
                ws.status = WorkstreamStatus::Blocked;
                blocked.push(ws.id.clone());
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
        // A warning, not a clean report: reported clean this exits `0` and
        // prints "nothing ready", which is indistinguishable from a board with
        // no work to do — and is how a diverged board sits untouched while
        // every run over it looks fine.
        let warning = Warning::new(
            "execute.plan_diverged",
            feature.to_string(),
            format!(
                "plan.md no longer matches the graph the board was approved against; \
                 {} {} blocked. Re-approve the board (`ivar feature execute prepare` then \
                 `approve`) or restore plan.md to the approved revision.",
                blocked.len(),
                if blocked.len() == 1 {
                    "workstream"
                } else {
                    "workstreams"
                }
            ),
        );
        return Ok(Report::with_warnings(
            TickOutcome {
                root: layout.root().to_path_buf(),
                feature,
                launched: Vec::new(),
                board,
                board_path,
                diverged: Some(PlanDivergence {
                    approved: board_fingerprint,
                    current: fp.clone(),
                    blocked,
                }),
            },
            vec![warning],
        ));
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
        // "Nothing was ready" is true here, unlike the divergence above — but
        // it is only the whole truth when the board is finished. A board that
        // still has work and cannot start any of it needs a human, and saying
        // so on exit `0` with the same sentence a completed board prints is
        // how a stalled board gets ticked forever.
        let stall = stalled_reason(&board, &feature);
        let outcome = TickOutcome {
            root: layout.root().to_path_buf(),
            feature,
            launched: Vec::new(),
            board,
            board_path,
            diverged: None,
        };
        return Ok(match stall {
            Some(warning) => Report::with_warnings(outcome, vec![warning]),
            None => Report::new(outcome),
        });
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
    let total = manifest.repos().len();
    for (index, repo) in manifest.repos().iter().enumerate() {
        ctx.progress().step(&pull::fetch_step(index, total, repo));
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
    ctx.progress().clear();

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
    // stray one — see `launch::AuditOutcome`'s "Two questions,
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
            contract: WriteContract::new(ws.write_contract.clone()),
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
        match event {
            events::TickEvent::Executor(event) => {
                events::apply_event(&mut board, &layout, &feature, &command_displays, event)?;
            }
            events::TickEvent::Warning(warning) => warnings.push(warning),
        }
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
            diverged: None,
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

/// Why a tick that launched nothing is not simply finished.
///
/// `None` for the one board that has nothing left to do — every workstream
/// `Done`. Otherwise work remains and none of it could start, which is always
/// a human's move, and the warning names which one. Ordered by what the human
/// would have to do first: an answer unblocks a workstream, an acknowledgement
/// resumes a paused one, and only when neither is outstanding is an unmet
/// dependency the thing left to explain.
fn stalled_reason(board: &ExecutionBoard, feature: &FeatureName) -> Option<Warning> {
    let ids = |status: WorkstreamStatus| -> Vec<&str> {
        board
            .graph
            .workstreams
            .iter()
            .filter(|ws| ws.status == status)
            .map(|ws| ws.id.as_str())
            .collect()
    };

    if board
        .graph
        .workstreams
        .iter()
        .all(|ws| ws.status == WorkstreamStatus::Done)
    {
        return None;
    }

    let blocked = ids(WorkstreamStatus::Blocked);
    if !blocked.is_empty() {
        return Some(Warning::new(
            "execute.awaiting_reply",
            feature.to_string(),
            format!(
                "nothing launched: {} stopped for a human. Answer with \
                 `ivar feature execute reply`.",
                blocked.join(", ")
            ),
        ));
    }

    let paused = ids(WorkstreamStatus::Paused);
    if !paused.is_empty() {
        return Some(Warning::new(
            "execute.awaiting_ack",
            feature.to_string(),
            format!(
                "nothing launched: {} paused by a revision. Acknowledge with \
                 `ivar feature execute ack-revision --workstream <id>`.",
                paused.join(", ")
            ),
        ));
    }

    // Waiting, but nothing to wait for that will ever arrive: the
    // dependencies are neither `Done` nor reachable from anything running.
    // A cycle in the graph is the usual cause, and it can only be edited out.
    let stuck: Vec<String> = board
        .graph
        .workstreams
        .iter()
        .filter(|ws| ws.status == WorkstreamStatus::Waiting)
        .map(|ws| {
            let unmet: Vec<&str> =
                ws.depends_on
                    .iter()
                    .filter(|dep| {
                        !board.graph.workstreams.iter().any(|other| {
                            other.id == **dep && other.status == WorkstreamStatus::Done
                        })
                    })
                    .map(String::as_str)
                    .collect();
            format!("{} waits on {}", ws.id, unmet.join(", "))
        })
        .collect();
    if stuck.is_empty() {
        return None;
    }
    Some(Warning::new(
        "execute.dependencies_unsatisfiable",
        feature.to_string(),
        format!(
            "nothing launched and nothing can: {}. No workstream is running to \
             satisfy them — check the graph for a dependency cycle.",
            stuck.join("; ")
        ),
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
