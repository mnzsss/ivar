//! The per-workstream launch half of `tick`: everything a worker thread
//! needs to materialise its own session, spawn the provider, and drain the
//! child's stream. Never touches the board — see `mod.rs`'s "Who spawns, who
//! owns the board".

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::feature::{Feature, WorkstreamDef, WriteContract};
use crate::domain::name::{FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::{SessionState, rfc3339_now};
use crate::error::Failure;
use crate::git::{self, Git};
use crate::harness::stream::{ExecutorEvent, parse_claude_line, parse_opencode_line};
use crate::harness::{Harness, guard};
use crate::infra::{fs, proc};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::super::session::view;
use super::events::{ExecutorTickEvent, TickEvent};

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
    /// Every write contract in this wave, unioned — what the post-run audit
    /// measures against. See "Auditing what the guard cannot see" below for
    /// why this is the wave's contract and not this workstream's own.
    pub(super) wave_contract: WriteContract,
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
        let _ = tx.send(TickEvent::Executor(ExecutorTickEvent {
            workstream_id: job.workstream_id.clone(),
            session_id: job.session_id.to_string(),
            event,
        }));
    };
    let send_warning = |warning| {
        let _ = tx.send(TickEvent::Warning(warning));
    };

    match view::materialise(
        &layout,
        &manifest,
        Some(&feature_record),
        job.provider,
        &job.view_dir,
    ) {
        Ok(report) => {
            for warning in report.warnings {
                send_warning(warning);
            }
        }
        Err(failure) => {
            send(ExecutorEvent::Failed {
                error: failure.to_string(),
            });
            return;
        }
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

    let (worktrees, baseline) = match audit_baseline(&layout, &feature_record) {
        Ok(pair) => pair,
        Err(failure) => {
            send(ExecutorEvent::Failed {
                error: format!(
                    "the write-contract audit could not read the worktrees, so this workstream was not launched: {failure}"
                ),
            });
            return;
        }
    };

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
        Ok(Some(0)) => match audit_write_contract(&worktrees, &job.wave_contract, &baseline) {
            Ok(None) => send(ExecutorEvent::Completed),
            Ok(Some(violation)) => send(ExecutorEvent::Failed { error: violation }),
            Err(failure) => send(ExecutorEvent::Failed {
                error: format!("the write-contract audit could not run: {failure}"),
            }),
        },
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

// -- Auditing what the guard cannot see ---------------------------------

/// The promoted worktrees a feature spans, each paired with the repo it came
/// from. The repo name is not decoration: a contract names its files
/// `<repo>/<path>`, so the audit needs it to build a path the contract can
/// match at all — see [`audit_path`].
type FeatureWorktrees = Vec<(RepoName, Utf8PathBuf)>;

/// What a run started from: the commit each worktree was on, and every path
/// already diverging from it.
///
/// Both halves are needed because a run's changes can land in either place.
/// `dirty` alone — which is all this used to be — is the working tree's
/// divergence from *its own current* commit, and a `git commit` sets that to
/// empty: the writes are still on the branch, and the audit that reads only
/// the working tree sees a clean run. `heads` is the fixed point that keeps
/// the committed half visible; see [`changed_since`].
pub(super) struct AuditBaseline {
    /// Where each worktree's branch was before the child started, keyed by
    /// worktree path.
    heads: BTreeMap<Utf8PathBuf, String>,
    /// Paths already diverging at launch, as `<repo>/<path>` — what the run
    /// inherited from an earlier tick or an uncommitted human edit, and must
    /// not be blamed for.
    dirty: BTreeSet<Utf8PathBuf>,
}

/// The worktrees this workstream can reach, and what it inherited in them —
/// the audit measures what *this* run changed, not what was already there.
///
/// Read before the child exists, so nothing the child writes can land in its
/// own baseline. A failure here stops the launch rather than the run: a
/// workstream whose writes could never be audited should not have started.
fn audit_baseline(
    layout: &Layout,
    feature_record: &Feature,
) -> Result<(FeatureWorktrees, AuditBaseline), Failure> {
    let worktrees = feature_worktrees(layout, feature_record)?;
    let git = git::System;
    let mut heads = BTreeMap::new();
    for (_, worktree) in &worktrees {
        heads.insert(worktree.clone(), git.head_commit(worktree)?);
    }
    // Taken with the heads just recorded, so `dirty` is exactly what
    // `changed_since` would report for a run that did nothing — the two sets
    // are then differenced against each other on equal terms.
    let dirty = changed_since(&worktrees, &heads)?;
    Ok((worktrees, AuditBaseline { heads, dirty }))
}

/// The promoted worktrees this feature spans, as `(repo name, worktree
/// path)`, skipping any that is not on disk. A feature with no promoted repo
/// yields none, and everything below is then a no-op.
///
/// A worktree that cannot be *stat*ed refuses rather than being skipped: a
/// silently dropped worktree is a worktree the audit does not look at, which
/// is the one outcome this function must not produce quietly.
fn feature_worktrees(
    layout: &Layout,
    feature_record: &Feature,
) -> Result<FeatureWorktrees, Failure> {
    let mut worktrees = Vec::new();
    for repo in feature_record.promotions.keys() {
        let worktree = layout.repo_worktree(repo, &feature_record.branch);
        if fs::is_dir(&worktree)? {
            worktrees.push((repo.clone(), worktree));
        }
    }
    Ok(worktrees)
}

/// Every path across `worktrees` that now differs from what `heads` recorded —
/// the run's uncommitted divergence *and* whatever it committed on top of the
/// commit it started from.
///
/// # Why both halves
///
/// `git status` is a question about the working tree, and the audit needs to
/// ask a question about a *run*. The two coincide only for as long as the run
/// never touches git. They come apart the moment it commits — which is not an
/// edge case but the expected end state, since `deliver` counts a dirty
/// worktree as a blocker, so somebody has to commit and the executor is the
/// one holding the shell. A run that did exactly what the pipeline wanted
/// therefore left the working tree clean, and an audit reading only the
/// working tree passed it for that reason rather than in spite of it: every
/// stray write the run committed alongside its real work went unreported.
///
/// Diffing `<head at launch>..HEAD` closes that, and closes it for the other
/// git actions too, because it compares two *trees* rather than walking the
/// commits between them. Ten commits, an amend, a rebase, a `reset --hard`
/// onto another commit, a `switch` to another branch — each leaves HEAD
/// somewhere with a diff from where the run began, and none of them need the
/// reflog reasoning a commit walk would.
///
/// A run that *reverts* what it inherited lands back at the baseline, so its
/// change set is not larger but smaller. That is not this function's problem
/// to solve — it reports the set, and [`audit_write_contract`] takes the difference in
/// both directions.
///
/// Paths come back as `<repo>/<path>`, the shape
/// [`WriteContract::allows`](crate::domain::feature::WriteContract::allows)
/// already arbitrates for the guard — a relative glob matches at any depth, so
/// the same contract decides the same way here as it does at the tool
/// boundary.
fn changed_since(
    worktrees: &FeatureWorktrees,
    heads: &BTreeMap<Utf8PathBuf, String>,
) -> Result<BTreeSet<Utf8PathBuf>, Failure> {
    let git = git::System;
    let mut changed = BTreeSet::new();
    for (repo, worktree) in worktrees {
        for relative in git.changed_paths(worktree)? {
            changed.insert(audit_path(repo, &relative));
        }
        // A worktree with no recorded head is one that appeared after the
        // baseline was taken; there is no fixed point to diff it against, and
        // its uncommitted half above is all this can honestly report.
        if let Some(head) = heads.get(worktree) {
            for relative in git.paths_committed_since(worktree, head)? {
                changed.insert(audit_path(repo, &relative));
            }
        }
    }
    Ok(changed)
}

/// One changed path in the shape a contract is written in: `<repo>/<path
/// within the repo>`.
///
/// This is why the audit carries repo names at all. A contract names its
/// files under the repo because that is the shape of a session view dir, and
/// the view dir is what the guard arbitrates at the tool boundary. A worktree
/// is laid out `<repo>/<branch>/<path>`, so a path joined onto its worktree
/// root carries a branch segment no contract mentions, and
/// [`WriteContract::allows`] — which anchors a relative glob with `ends_with`
/// — then matches nothing at all. Not "misses a violation": reports every new
/// write as one, in every repo, including the writes the workstream was
/// launched to make.
pub(super) fn audit_path(repo: &RepoName, relative: &Utf8Path) -> Utf8PathBuf {
    Utf8PathBuf::from(repo.as_str()).join(relative)
}

/// What this run wrote that no contract in the wave allows, as a ready-made
/// failure message — `None` when the run stayed inside the lines.
///
/// # Why this exists at all
///
/// The execution guard ([`guard`]) is a `PreToolUse` hook registered for
/// `Write|Edit|MultiEdit|NotebookEdit`: the tools whose call carries a path it
/// can arbitrate. `Bash` carries a *command*, and a command has no path to
/// check — a shell one-liner, a heredoc into `python3 -`, a formatter, a code
/// generator, all write without the guard ever being consulted. That is not a
/// gap that can be closed at the tool boundary, because deciding what a shell
/// command writes means deciding what a program does.
///
/// So it is closed here instead, at the only place that cannot be talked past:
/// the filesystem, after the fact. This observes effects rather than
/// intentions, which also catches the writes no pre-check could have predicted
/// — the formatter that reached outside its contract, the generator that
/// emitted one file too many.
///
/// It **detects**, it does not prevent: the bytes are already on disk when
/// this runs. What it guarantees is that they cannot pass unnoticed as a
/// completed workstream. Nothing is reverted — an audit that deleted an
/// agent's work on suspicion would be a worse failure than the one it guards
/// against.
///
/// # Why the wave's contract, not this workstream's
///
/// A tick launches a wave of workstreams in parallel against the *same*
/// worktrees. A path this workstream never touched, written by a sibling
/// running beside it, is indistinguishable here from one it wrote itself —
/// `git status` records the change, not its author. Measuring against this
/// workstream's own contract alone would therefore blame it for every
/// sibling's legitimate work.
///
/// The wave's union is the sharpest line that stays true: a path no contract
/// in the wave allows is a violation whoever wrote it, and a wave of one — the
/// common case — measures exactly the workstream's own contract. What this
/// deliberately does not catch is one workstream writing inside *another's*
/// contract; attributing that needs an author, which the filesystem does not
/// record. The guard still refuses it for the tools it covers.
fn audit_write_contract(
    worktrees: &FeatureWorktrees,
    wave_contract: &WriteContract,
    baseline: &AuditBaseline,
) -> Result<Option<String>, Failure> {
    let after = changed_since(worktrees, &baseline.heads)?;

    // The same difference, taken both ways. A run changes the set of diverging
    // paths in two directions, and only one of them is an added file: `git
    // checkout -- .`, `git reset --hard` and `git stash` all make divergence
    // *disappear*, and an audit that only asks what grew reads the destruction
    // of an inherited edit as a run that stayed inside the lines.
    let written = contract_violations(wave_contract, &baseline.dirty, &after);
    let reverted = contract_violations(wave_contract, &after, &baseline.dirty);

    Ok(violation_report(&written, &reverted))
}

/// The paths in `after` that `baseline` did not already hold and that
/// `wave_contract` does not allow.
///
/// Called both ways round by [`audit_write_contract`]: with `(baseline, after)` it names
/// what the run wrote where it may not, and with the two swapped it names what
/// the run *stopped* diverging that it may not have touched — a revert. The
/// question is the same one in both directions ("which side of this difference
/// does no contract cover"), so it is asked with the same function rather than
/// a near-copy that could drift from it.
pub(super) fn contract_violations(
    wave_contract: &WriteContract,
    baseline: &BTreeSet<Utf8PathBuf>,
    after: &BTreeSet<Utf8PathBuf>,
) -> Vec<Utf8PathBuf> {
    after
        .difference(baseline)
        .filter(|path| !wave_contract.allows(path))
        .cloned()
        .collect()
}

/// How many violating paths a failure message names before it stops counting.
/// A runaway generator can produce thousands; a journal entry that carries all
/// of them is one nobody reads.
const VIOLATIONS_NAMED: usize = 20;

/// The failure a violating run reports, or `None` when it violated nothing in
/// either direction.
///
/// Both halves are reported when both happened: a run that wrote where it may
/// not *and* threw away an inherited edit did two things, and naming one would
/// leave the reader to discover the other.
fn violation_report(written: &[Utf8PathBuf], reverted: &[Utf8PathBuf]) -> Option<String> {
    let mut parts = Vec::new();
    if !written.is_empty() {
        parts.push(violation_message(written));
    }
    if !reverted.is_empty() {
        parts.push(revert_message(reverted));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The failure a violating run reports, naming what it wrote and why that is
/// being reported at the end rather than refused at the time.
///
/// Every violation is already `<repo>/<path>` — see [`audit_path`] — which is
/// both what the contract was compared against and what a reader recognises.
fn violation_message(violations: &[Utf8PathBuf]) -> String {
    format!(
        "write contract violated: {} path(s) changed that no workstream in this wave may write — {}. \
         The execution guard arbitrates Write, Edit, MultiEdit and NotebookEdit; a shell command carries no \
         path for it to check and writes past it, so this audit is what sees that. Nothing was reverted by ivar.",
        violations.len(),
        named_paths(violations),
    )
}

/// The failure a run that *un*wrote something reports.
///
/// Worth its own sentence rather than being folded into
/// [`violation_message`]: the paths named here are ones that diverged before
/// the run and do not any more, so what a reader has to go looking for is
/// content that no longer exists on disk — a different recovery from an
/// unwanted file that does.
///
/// Only the run's own commits can restore it, and it may have none: an
/// uncommitted edit reverted with `git checkout --` is gone from the
/// repository entirely.
fn revert_message(reverted: &[Utf8PathBuf]) -> String {
    format!(
        "write contract violated: {} path(s) diverged before this run and no longer do, and no workstream in \
         this wave may write them — {}. Something the run did (`git checkout --`, `git reset --hard`, `git \
         stash`) threw away changes it inherited rather than producing its own. Content that was never \
         committed is not recoverable from this repository.",
        reverted.len(),
        named_paths(reverted),
    )
}

/// Up to [`VIOLATIONS_NAMED`] paths, with a count of whatever is left over.
fn named_paths(paths: &[Utf8PathBuf]) -> String {
    let named: Vec<String> = paths
        .iter()
        .take(VIOLATIONS_NAMED)
        .map(Utf8PathBuf::to_string)
        .collect();
    let rest = paths.len().saturating_sub(named.len());
    if rest == 0 {
        named.join(", ")
    } else {
        format!("{} (and {rest} more)", named.join(", "))
    }
}
