//! Unit tests for `crate::action::execute::tick`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;

use super::*;
use crate::action::execute::approve::{self as approve_action, ApproveInput};
use crate::action::execute::prepare::{self as prepare_action, PrepareInput};
use crate::action::execute::reply::{self as reply_action, ReplyInput};
use crate::action::feature::create::{self as feature_create, CreateInput as FeatureCreateInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::feature::WriteContract;
use crate::domain::name::RepoName;
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
    assert_eq!(persisted(&root).status, ExecutionStatus::Completed);

    // Session→workstream link recorded.
    let on_disk = persisted(&root);
    assert_eq!(on_disk.sessions.len(), 1);
    let (sess_id, ws_id) = on_disk.sessions.iter().next().unwrap();
    assert_eq!(ws_id, "ws-b");
    assert!(!sess_id.is_empty());
}

#[test]
fn tick_warns_when_executor_cannot_materialise_canonical_instructions() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());
    fs::remove_file(&root.join("HALL.md")).unwrap();

    let _stub = PathStub::install("claude", "exit 0");
    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.warnings.len(), 1);
    assert_eq!(
        report.warnings[0].code,
        "instructions.canonical_unavailable"
    );
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

/// A graph whose dependencies can never be satisfied is not a quiet no-op.
///
/// Both workstreams below wait on each other, so no tick will ever launch
/// either — and the board is not finished. Reported clean, that is exit `0`
/// and "nothing ready", the same answer a completed board gives, and the cycle
/// can only be found by reading the graph by hand.
#[test]
fn a_tick_that_can_never_launch_anything_says_so() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
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

    assert!(
        report.value.launched.is_empty(),
        "circular deps: nothing should launch"
    );
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(
        report.warnings[0].code,
        "execute.dependencies_unsatisfiable"
    );
    assert!(report.warnings[0].what.contains("ws-a waits on ws-b"));
    assert!(report.warnings[0].what.contains("ws-b waits on ws-a"));
}

/// The one board that is genuinely nothing to do: every workstream `Done`.
/// This is the case the warnings above must not fire on, or every finished
/// board reports a problem forever.
#[test]
fn a_tick_on_a_finished_board_is_clean() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    for ws in &mut board.graph.workstreams {
        ws.status = WorkstreamStatus::Done;
    }
    board.write(&layout, &feature).unwrap();

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert!(report.is_clean(), "a finished board must not warn");
    assert!(report.value.launched.is_empty());
}

/// A board stopped on a question needs an answer, not another tick. Saying
/// "nothing ready" on exit `0` is what let a blocked board be ticked in a loop
/// that could never move it.
#[test]
fn a_tick_on_a_board_waiting_for_a_human_says_which_workstream() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let mut board = ExecutionBoard::read(&layout, &feature).unwrap().unwrap();
    board.graph.workstreams[0].status = WorkstreamStatus::Blocked;
    board.graph.workstreams[1].status = WorkstreamStatus::Blocked;
    board.write(&layout, &feature).unwrap();

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "execute.awaiting_reply");
    assert!(report.warnings[0].what.contains("ws-a"));
    assert!(report.warnings[0].what.contains("reply"));
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

// -- wave handoff --------------------------------------------------------

/// A tick launches one wave and blocks until it finishes. The board it
/// leaves behind has to be tickable again, or every wave after the first is
/// stranded: `tick` demands `approved`, and a board parked at `running`
/// refuses every command that could move it — `tick` (not approved),
/// `approve` (not awaiting approval) and `reply` (not blocked) alike.
#[test]
fn a_finished_wave_returns_the_board_to_approved_so_the_next_can_launch() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "exit 0");

    let first = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(first.value.launched, vec!["ws-a"]);

    let after_first = persisted(&root);
    assert_eq!(
        after_first.status,
        ExecutionStatus::Approved,
        "a wave that finished with work still waiting must leave the board tickable"
    );
    assert!(
        after_first
            .journal
            .iter()
            .any(|entry| entry.kind == "wave.completed"),
        "the handoff back to `approved` must be journalled: {:?}",
        after_first.journal
    );

    // The next wave launches from that same board — no re-approval, no
    // re-prepare.
    let second = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(second.value.launched, vec!["ws-b"]);

    let after_second = persisted(&root);
    assert_eq!(
        after_second.status,
        ExecutionStatus::Completed,
        "a board whose every workstream is done is completed, not running"
    );
    assert!(
        after_second
            .journal
            .iter()
            .any(|entry| entry.kind == "board.completed"),
        "the completion must be journalled: {:?}",
        after_second.journal
    );
}

/// A blocked wave stays blocked: settling after the fold loop must not
/// launder a board that needs a human back into `approved`.
#[test]
fn a_blocked_wave_stays_blocked_after_the_tick_settles() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "exit 7");

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let on_disk = persisted(&root);
    assert_eq!(on_disk.status, ExecutionStatus::Blocked);
    assert_eq!(on_disk.blocked_by.as_deref(), Some("ws-a"));
}

/// The whole recovery loop, end to end: a workstream blocks, a human
/// replies, and the next tick relaunches it — with the answer in its prompt.
/// A relaunch that dropped the answer would hand the agent the exact prompt
/// that produced the question and get the same question back, forever.
#[test]
fn a_replied_workstream_relaunches_with_the_answer_in_its_prompt() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

    // Wave 1: the child dies, so the board blocks on `ws-a`.
    {
        let _stub = PathStub::install("claude", "exit 3");
        tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
    }
    let blocked = persisted(&root);
    assert_eq!(blocked.status, ExecutionStatus::Blocked);
    let session = blocked
        .sessions
        .iter()
        .find(|(_, workstream)| workstream.as_str() == "ws-a")
        .map(|(id, _)| id.clone())
        .unwrap();

    reply_action::reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some(session),
            message: "use the v2 endpoint".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        persisted(&root).status,
        ExecutionStatus::Approved,
        "a reply must leave the board tickable"
    );

    // Wave 2: capture the prompt the relaunched child is handed. `claude`
    // takes it as the argument to `-p`, so `$2` is the prompt itself.
    let (_sentinel_guard, sentinel_dir) = crate::test_support::utf8_temp_dir();
    let sentinel = sentinel_dir.join("prompt.txt");
    let _stub = PathStub::install("claude", &format!("printf '%s' \"$2\" > '{sentinel}'"));

    let report = tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(report.value.launched, vec!["ws-a"]);

    let prompt_text = fs::read_text(&sentinel).unwrap().unwrap();
    assert!(
        prompt_text.contains("use the v2 endpoint"),
        "the relaunched prompt must carry the human's answer: {prompt_text}"
    );
}

// -- the post-run write-contract audit ----------------------------------

/// The audit's whole reason for existing: the execution guard is a
/// `PreToolUse` hook on `Write|Edit|MultiEdit|NotebookEdit`, and `Bash`
/// carries a command rather than a path, so a shell write reaches the disk
/// without the guard ever being asked. This is what sees it afterwards.
#[test]
fn a_path_outside_every_contract_in_the_wave_is_a_violation() {
    // A contract names directories with a trailing `/` and files by name —
    // the shapes `WriteContract::allows` arbitrates. The paths are
    // `<repo>/<path>`, which is what `launch::audit_path` builds and what a
    // contract is written against.
    let contract = WriteContract::new(vec!["src/a/".to_owned(), "src/b/".to_owned()]);
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [
        Utf8PathBuf::from("api-main/src/a/one.rs"),
        Utf8PathBuf::from("api-main/src/b/two.rs"),
        Utf8PathBuf::from("api-main/src/elsewhere/three.rs"),
    ]
    .into_iter()
    .collect();

    let violations = launch::contract_violations(&contract, &baseline, &after);

    assert_eq!(
        violations,
        vec![Utf8PathBuf::from("api-main/src/elsewhere/three.rs")]
    );
}

/// A wave shares its worktrees, so a sibling's legitimate write shows up in
/// this workstream's `git status` exactly like its own stray one. Measuring
/// against the wave's union is what keeps the audit from blaming `ws-a` for
/// the file `ws-b` was launched to write.
#[test]
fn a_sibling_workstream_s_own_files_are_not_reported_as_violations() {
    // The union `tick` builds: `ws-a` owns `src/a`, `ws-b` owns `src/b`.
    let wave = WriteContract::new(vec!["src/a/".to_owned(), "src/b/".to_owned()]);
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [Utf8PathBuf::from("api-main/src/b/two.rs")]
        .into_iter()
        .collect();

    assert!(launch::contract_violations(&wave, &baseline, &after).is_empty());
}

/// What was already dirty when the workstream started is not what it wrote —
/// an uncommitted human edit, or an earlier tick's work, must not fail the
/// next workstream that runs beside it.
#[test]
fn what_was_already_dirty_before_the_run_is_not_a_violation() {
    let contract = WriteContract::new(vec!["src/a/".to_owned()]);
    let inherited = Utf8PathBuf::from("api-main/notes.md");
    let baseline: BTreeSet<Utf8PathBuf> = [inherited.clone()].into_iter().collect();
    let after: BTreeSet<Utf8PathBuf> = [inherited].into_iter().collect();

    assert!(launch::contract_violations(&contract, &baseline, &after).is_empty());
}

/// An empty contract allows nothing — the domain's own default-deny — so a
/// workstream with no contract that writes anything at all is caught.
#[test]
fn an_empty_contract_allows_nothing() {
    let contract = WriteContract::new(Vec::new());
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [Utf8PathBuf::from("api-main/src/a/one.rs")]
        .into_iter()
        .collect();

    assert_eq!(
        launch::contract_violations(&contract, &baseline, &after).len(),
        1
    );
}

/// A real write contract names its files `<repo>/<path>`, the shape of a
/// session view dir. Matching that against a path built from a worktree root
/// wedges the branch segment in the middle, `ends_with` never fires, and every
/// new write in every repo is reported as a violation.
///
/// The `src/a/` contracts the tests above use match at any depth, so none of
/// them can see this.
#[test]
fn a_path_the_contract_names_by_repo_is_matched_against_the_repo_s_own_shape() {
    let repo = RepoName::new("gaio-backend").unwrap();
    let relative = Utf8PathBuf::from("packages/console/src/workflows/repositories/workflow.ts");

    let contract = WriteContract::new(vec![
        "gaio-backend/packages/console/src/workflows/repositories/workflow.ts".to_owned(),
    ]);
    let baseline = BTreeSet::new();
    let after: BTreeSet<Utf8PathBuf> = [launch::audit_path(&repo, &relative)].into_iter().collect();

    assert!(
        launch::contract_violations(&contract, &baseline, &after).is_empty(),
        "the workstream's own contracted file must not be reported as a violation"
    );
}

// -- what the audit's oracle cannot see ----------------------------------
//
// Both tests below need a *real* promoted worktree. Without one,
// `launch::feature_worktrees` yields nothing and the whole post-run audit is
// a no-op — which is why every audit test above it is a pure unit test on
// `contract_violations` and none of them can see either bug.

/// A hall with a real seeded repo promoted into the feature, plus the plan and
/// the prepared+approved board `approved_board` builds.
///
/// The contract is `src/a/` (trailing slash) rather than `src/a`: a bare
/// relative name only matches a path *ending* in it, so `api/src/a/one.rs`
/// would not match and the test would be measuring the matcher, not the audit.
fn approved_board_with_worktree() -> (tempfile::TempDir, Utf8PathBuf) {
    use crate::action::feature::promote::{self, PromoteInput};
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::seeded_repo;

    const WORKTREE_GRAPH_JSON: &str = r#"{
        "workstreams": [
            {
                "id": "ws-a",
                "title": "A",
                "operations": ["op-a"],
                "depends_on": [],
                "write_contract": ["src/a/"]
            }
        ]
    }"#;

    const WORKTREE_PLAN_TEXT: &str = "# Plan\n\
        \n\
        ## Operation details\n\
        \n\
        **op-a** — Do the first thing.\n\
        \n\
        ## Operations\n\
        \n\
        ### ws-a\n\
        - op-a\n";

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

    let origin = seeded_repo(&root.join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

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
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    fs::write_text(&root.join("plans/checkout/plan.md"), WORKTREE_PLAN_TEXT).unwrap();
    let graph = root.join("graph.json");
    fs::write_text(&graph, WORKTREE_GRAPH_JSON).unwrap();
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

/// The worktree this feature's one promoted repo materialised into.
fn feature_worktree(root: &Utf8PathBuf) -> Utf8PathBuf {
    root.join(".ivar/repos/api/checkout")
}

/// A `git` invocation with a fixed identity, as a shell fragment a stub
/// executor can run — a machine with no global `user.email` cannot commit at
/// all, and that failure is opaque.
fn stub_git(worktree: &Utf8PathBuf) -> String {
    format!(
        "git -C '{worktree}' -c user.name='stub' -c user.email='stub@ivar.invalid' \
         -c commit.gpgsign=false"
    )
}

/// The audit's oracle is `git status --porcelain`: what diverges from the
/// index right now. An executor that commits — which is what the rest of the
/// pipeline needs it to do, since `deliver` counts a dirty worktree as a
/// blocker — empties that oracle. The stray file is on the branch, in a
/// commit, and the audit reports a clean run.
#[test]
fn a_run_that_commits_its_stray_writes_is_still_a_violation() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);
    let git = stub_git(&worktree);

    let _stub = PathStub::install(
        "claude",
        &format!(
            "mkdir -p '{worktree}/src/a'\n\
             printf 'contracted\\n' > '{worktree}/src/a/one.rs'\n\
             printf 'stray\\n' > '{worktree}/elsewhere.rs'\n\
             {git} add -A >/dev/null 2>&1\n\
             {git} commit -m 'work' >/dev/null 2>&1\n\
             exit 0\n"
        ),
    );

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    let ws = &board.graph.workstreams[0];
    assert_ne!(
        ws.status,
        WorkstreamStatus::Done,
        "a run that wrote `elsewhere.rs` — which no contract in the wave allows — \
         must not reach Done just because it committed the evidence"
    );
    assert!(
        board
            .journal
            .iter()
            .any(|entry| entry.message.contains("write contract violated")),
        "the audit must report the committed stray write; journal: {:?}",
        board
            .journal
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>()
    );
}

/// `Done` is derived from the child's exit code and nothing else, so a session
/// that ran, wrote not one byte and exited cleanly is indistinguishable on the
/// board from one that did the whole job. The audit already computes what the
/// run changed; it just never asks whether any of it was inside the contract.
#[test]
fn a_run_that_produced_nothing_does_not_reach_done() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());

    let _stub = PathStub::install("claude", "exit 0");

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    let ws = &board.graph.workstreams[0];
    assert_ne!(
        ws.status,
        WorkstreamStatus::Done,
        "a session that changed nothing under its write contract produced nothing — \
         `Done` claims work that does not exist, and every workstream depending on \
         this one then launches against it"
    );
}

/// The control for the two tests above: the *same* stray write, left
/// uncommitted, is caught. What defeats the audit is not the write, the
/// worktree or the contract — it is the `git commit` in between.
#[test]
fn an_uncommitted_stray_write_is_caught() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);

    let _stub = PathStub::install(
        "claude",
        &format!(
            "mkdir -p '{worktree}/src/a'\n\
             printf 'contracted\\n' > '{worktree}/src/a/one.rs'\n\
             printf 'stray\\n' > '{worktree}/elsewhere.rs'\n\
             exit 0\n"
        ),
    );

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(board.graph.workstreams[0].status, WorkstreamStatus::Blocked);
    assert!(
        board
            .journal
            .iter()
            .any(|entry| entry.message.contains("write contract violated"))
    );
}

/// The escape the per-run production check needs to survive a relaunch.
///
/// A workstream that blocked on a question is relaunched from scratch, from a
/// baseline that already contains what its first run wrote. Judged per run,
/// the second run changed nothing new and would block again — then be replied
/// to, relaunched, and block again, forever. The journal is what breaks that
/// loop: the first run's `produced` entry is still on the board.
#[test]
fn a_relaunch_of_a_workstream_that_already_produced_reaches_done() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);

    // Wave 1: writes its contracted file, then dies — blocked, but productive.
    {
        let _stub = PathStub::install(
            "claude",
            &format!(
                "mkdir -p '{worktree}/src/a'\n\
                 printf 'work\\n' > '{worktree}/src/a/one.rs'\n\
                 exit 3\n"
            ),
        );
        tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
    }

    let blocked = persisted(&root);
    assert_eq!(blocked.status, ExecutionStatus::Blocked);
    assert!(
        blocked
            .journal
            .iter()
            .any(|entry| entry.kind == "produced" && entry.workstream == "ws-a"),
        "a run that wrote its contracted file must record that it produced, \
         even though the child then failed"
    );
    let session = blocked
        .sessions
        .iter()
        .find(|(_, workstream)| workstream.as_str() == "ws-a")
        .map(|(id, _)| id.clone())
        .unwrap();

    reply_action::reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some(session),
            message: "carry on".to_owned(),
        },
    )
    .unwrap();

    // Wave 2: nothing left to do, so it changes nothing and exits cleanly.
    let _stub = PathStub::install("claude", "exit 0");
    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(
        board.graph.workstreams[0].status,
        WorkstreamStatus::Done,
        "the workstream produced in its first run; a relaunch with nothing left \
         to do must not block it again"
    );
}

/// The other side of the same rule: a workstream that has never produced does
/// not earn a `done` by being relaunched. The escape is the *journal entry*,
/// not the relaunch.
#[test]
fn a_relaunch_of_a_workstream_that_never_produced_still_blocks() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());

    {
        let _stub = PathStub::install("claude", "exit 3");
        tick(
            &ctx,
            TickInput {
                feature: "checkout".to_owned(),
            },
        )
        .unwrap();
    }
    let session = persisted(&root)
        .sessions
        .iter()
        .find(|(_, workstream)| workstream.as_str() == "ws-a")
        .map(|(id, _)| id.clone())
        .unwrap();
    reply_action::reply(
        &ctx,
        ReplyInput {
            feature: Some("checkout".to_owned()),
            session: Some(session),
            message: "carry on".to_owned(),
        },
    )
    .unwrap();

    let _stub = PathStub::install("claude", "exit 0");
    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(board.graph.workstreams[0].status, WorkstreamStatus::Blocked);
    assert!(
        board
            .journal
            .iter()
            .any(|entry| entry.kind == "session.unproductive")
    );
}

/// A run that does the job reaches `Done` and says what it produced — the
/// positive case the two refusals above are measured against.
#[test]
fn a_run_that_writes_its_contracted_files_reaches_done_and_records_them() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);

    let _stub = PathStub::install(
        "claude",
        &format!(
            "mkdir -p '{worktree}/src/a'\n\
             printf 'work\\n' > '{worktree}/src/a/one.rs'\n\
             exit 0\n"
        ),
    );
    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(board.graph.workstreams[0].status, WorkstreamStatus::Done);
    let produced = board
        .journal
        .iter()
        .find(|entry| entry.kind == "produced")
        .expect("a productive run records what it produced");
    assert!(
        produced.message.contains("api/src/a/one.rs"),
        "the entry must name the contracted path: {}",
        produced.message
    );
}

/// Committing is the expected end state — `deliver` counts a dirty worktree as
/// a blocker — so the production half of the audit has to survive it too. A
/// run whose only trace is a commit still produced.
#[test]
fn a_run_that_commits_its_contracted_work_still_counts_as_producing() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);
    let git = stub_git(&worktree);

    let _stub = PathStub::install(
        "claude",
        &format!(
            "mkdir -p '{worktree}/src/a'\n\
             printf 'work\\n' > '{worktree}/src/a/one.rs'\n\
             {git} add -A >/dev/null 2>&1\n\
             {git} commit -m 'work' >/dev/null 2>&1\n\
             exit 0\n"
        ),
    );
    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(
        board.graph.workstreams[0].status,
        WorkstreamStatus::Done,
        "work that was committed is still work; journal: {:?}",
        board
            .journal
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>()
    );
}

// -- reverting what the run inherited ------------------------------------

/// A run does not only add paths. `git checkout -- .`, `git reset --hard` and
/// `git stash` all *remove* divergence, and an audit that only ever asks what
/// grew reads the destruction of a human's uncommitted edit as a run that
/// stayed inside the lines.
#[test]
fn reverting_an_inherited_edit_outside_the_contract_is_a_violation() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);
    let git = stub_git(&worktree);

    // An uncommitted human edit the run inherits, outside every contract in
    // the wave (`src/a/`).
    fs::write_text(&worktree.join("README.md"), "a human was working here\n").unwrap();

    let _stub = PathStub::install(
        "claude",
        &format!(
            "mkdir -p '{worktree}/src/a'\n\
             printf 'work\\n' > '{worktree}/src/a/one.rs'\n\
             {git} checkout -- README.md >/dev/null 2>&1\n\
             exit 0\n"
        ),
    );

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(
        board.graph.workstreams[0].status,
        WorkstreamStatus::Blocked,
        "the run threw away an edit no contract in the wave covers"
    );
    let reported = board
        .journal
        .iter()
        .find(|entry| {
            entry
                .message
                .contains("diverged before this run and no longer do")
        })
        .map(|entry| entry.message.clone())
        .unwrap_or_else(|| {
            panic!(
                "the audit must report the reverted path; journal: {:?}",
                board
                    .journal
                    .iter()
                    .map(|entry| entry.message.as_str())
                    .collect::<Vec<_>>()
            )
        });
    assert!(
        reported.contains("api/README.md"),
        "the entry must name what was thrown away: {reported}"
    );
}

/// The false positive this must not have: committing an inherited edit also
/// takes it out of `git status`, and a run that commits is a run doing what
/// the pipeline asked. The commit half of the change set is what tells the
/// two apart.
#[test]
fn committing_an_inherited_edit_is_not_a_revert() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);
    let git = stub_git(&worktree);

    fs::write_text(&worktree.join("README.md"), "a human was working here\n").unwrap();

    let _stub = PathStub::install(
        "claude",
        &format!(
            "mkdir -p '{worktree}/src/a'\n\
             printf 'work\\n' > '{worktree}/src/a/one.rs'\n\
             {git} add -A >/dev/null 2>&1\n\
             {git} commit -m 'work' >/dev/null 2>&1\n\
             exit 0\n"
        ),
    );

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(
        board.graph.workstreams[0].status,
        WorkstreamStatus::Done,
        "committing an inherited edit preserves it; journal: {:?}",
        board
            .journal
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>()
    );
}

/// The limit of the revert check, pinned so nobody tightens it into a false
/// positive: a path *inside* the wave's contracts may legitimately be reverted
/// by the workstream that owns it, and git records that a file stopped
/// diverging, never who made it stop. Attribution needs an author the
/// filesystem does not have — exactly the reason the addition half measures
/// against the wave's union too.
#[test]
fn reverting_a_path_inside_the_wave_contract_is_not_a_violation() {
    let (_guard, root) = approved_board_with_worktree();
    let ctx = Ctx::new(root.clone());
    let worktree = feature_worktree(&root);
    let git = stub_git(&worktree);

    // An inherited edit *inside* the contract, committed so it can be reverted
    // by path — and committed before the run, so it is not what this run did.
    std::fs::create_dir_all(worktree.join("src/a")).unwrap();
    fs::write_text(&worktree.join("src/a/inherited.rs"), "first pass\n").unwrap();
    for args in [
        format!("{git} add -A"),
        format!("{git} commit -m inherited"),
    ] {
        assert!(
            std::process::Command::new("sh")
                .arg("-c")
                .arg(&args)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write_text(&worktree.join("src/a/inherited.rs"), "second pass\n").unwrap();

    let _stub = PathStub::install(
        "claude",
        &format!(
            "printf 'work\\n' > '{worktree}/src/a/one.rs'\n\
             {git} checkout -- src/a/inherited.rs >/dev/null 2>&1\n\
             exit 0\n"
        ),
    );

    tick(
        &ctx,
        TickInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let board = persisted(&root);
    assert_eq!(
        board.graph.workstreams[0].status,
        WorkstreamStatus::Done,
        "a revert inside the wave's own contracts is not attributable and is not \
         reported; journal: {:?}",
        board
            .journal
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>()
    );
}

/// A tick that blocked every waiting workstream must not report a clean no-op.
///
/// The divergence path marks each `Waiting` workstream `Blocked` and journals
/// it, but it used to return a warning-free `Report` — which `exit_code_for`
/// renders as `0` and `write_human` rendered as "nothing ready ... no
/// workstreams to launch". Both were false: something was ready, the plan moved
/// under it, and a human has to re-approve before anything launches again. A
/// caller driving `tick` on its exit code read success and moved on, which is
/// how a board sat blocked while every run over it looked fine.
#[test]
fn tick_that_blocks_a_diverged_plan_does_not_report_success() {
    let (_guard, root) = approved_board();
    let ctx = Ctx::new(root.clone());

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

    assert!(
        !report.is_clean(),
        "a tick that blocked every waiting workstream must not exit clean"
    );
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "execute.plan_diverged");

    // And the human surface must say what happened, not "nothing ready".
    let mut rendered = Vec::new();
    report.value.write_human(&mut rendered).unwrap();
    let rendered = String::from_utf8(rendered).unwrap();
    assert!(
        rendered.contains("diverged"),
        "the human output must name the divergence, got: {rendered}"
    );
    assert!(
        !rendered.contains("nothing ready"),
        "the human output must not claim nothing was ready, got: {rendered}"
    );
}
