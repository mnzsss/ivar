//! The setup-script half of `ivar sync`: run each repo's `.ivar/setups/<repo>.sh`
//! when it needs running, streamed, with the `IVAR_*` environment contract.

use camino::Utf8Path;

use crate::action::{SETUP_INTERPRETER, worktree_env};
use crate::error::{Failure, FixAction};
use crate::git::{self};
use crate::infra::{fs, hash, proc};
use crate::store::layout::Layout;
use crate::store::manifest::Repo;
use crate::store::setup_receipt::Receipt;

use super::{Change, Entry};

pub(crate) fn run_setup_script(
    git: &impl git::Git,
    layout: &Layout,
    repo: &Repo,
    worktree: &Utf8Path,
    surface: &str,
    force: bool,
) -> Result<Option<Entry>, Failure> {
    let script = layout.setup_script(repo.name());
    if !fs::is_file(&script)? {
        return Ok(None);
    }

    // Content, not mtime: a `git pull` that rewrites the script byte-identical
    // must not trigger a re-run, and one that changes a line must.
    let fingerprint = hash::file(&script)?;
    let git_dir = git.worktree_git_dir(worktree)?;
    let receipt = Receipt::read(&git_dir);

    if !Receipt::should_run(receipt.as_ref(), &fingerprint, force) {
        return Ok(Some(
            Entry::new(surface, "setup script", Change::Unchanged)
                .detail("already run for this version of the script"),
        ));
    }
    let first_run = receipt.is_none();

    // A default worktree a session has read-only-guarded is still the worktree
    // the script has to write into — bootstrapping this checkout is its whole
    // job. Lift the guard for the run and put it back after, the way a git
    // mutation does (`action::repo::pull::refresh_default`). Nothing to lift on
    // a feature worktree: promotion is what makes it writable in the first
    // place.
    let guarded = fs::unix_mode(worktree)?.is_some_and(|mode| mode & 0o222 == 0);
    if guarded {
        fs::restore_write_bits(worktree)?;
    }

    let run = proc::inherit(&setup_command(layout, repo, worktree, &script));

    // Re-guarded before the run is judged: a worktree must not be left writable
    // because the script failed, or because spawning it did. Best-effort, like
    // the pull path — the run's result stands even if the chmod does not.
    if guarded {
        let _ = fs::clear_write_bits(worktree);
    }
    let code = run?;

    // Recorded before the exit code is judged, so a failed run is remembered as
    // failed — which is what makes the next sync retry it instead of skipping.
    Receipt::write(&git_dir, &Receipt::of_run(&fingerprint, code))?;

    if code != Some(0) {
        return Err(setup_script_failed(&script, code));
    }

    Ok(Some(Entry::new(
        surface,
        "setup script",
        if first_run {
            Change::Created
        } else {
            Change::Updated
        },
    )))
}

pub(crate) fn setup_command(
    layout: &Layout,
    repo: &Repo,
    worktree: &Utf8Path,
    script: &Utf8Path,
) -> proc::Command {
    worktree_env(
        proc::Command::new(SETUP_INTERPRETER)
            .arg(script.as_str())
            .cwd(worktree),
        layout,
        repo.name().as_str(),
        repo.default_branch().as_str(),
        worktree,
    )
    // `default`, never `feature`: this is the hall's own checkout of the
    // repo's default branch. Feature worktrees get theirs on promote.
    .env("IVAR_WORKTREE_KIND", "default")
    // `IVAR_FEATURE` is deliberately absent: there is no feature here. The
    // promote path sets it, and a script that reads it unguarded should fail
    // loudly on the default worktree rather than silently bootstrap the wrong
    // thing.
}

pub(crate) fn setup_script_failed(script: &Utf8Path, code: Option<i32>) -> Failure {
    let ended = match code {
        Some(code) => format!("exited {code}"),
        None => "was killed by a signal".to_owned(),
    };

    Failure::failed("sync.setup_script_failed", format!("`{script}` {ended}"))
        .expected("the setup script to exit 0")
        .actual(ended)
        .fix(FixAction::safe(
            "sync.read_setup_output",
            "Read the script's output above — it ran with its own stdout and stderr attached.",
        ))
        .fix(
            FixAction::safe(
                "sync.retry_setup",
                "Fix the script, then run `ivar sync` again — a failed run is always retried.",
            )
            .command("ivar sync"),
        )
}
