use clap::Subcommand;

/// The `ivar hall` surface: the hall as a git repository.
#[derive(Debug, Subcommand)]
pub enum HallCommand {
    /// Commit everything pending in the hall and push it.
    ///
    /// Commits every change in the hall's working tree — modified, new or
    /// deleted, staged or not — as `chore(hall): Sync pending hall changes`,
    /// then pushes the current branch to its upstream. `.gitignore` is
    /// respected, so a `.env` or anything under `.ivar/` other than
    /// `.ivar/skills/` never enters a commit. Nothing is forced and nothing
    /// waits for input.
    ///
    /// A push the remote refuses because it holds newer commits runs
    /// `git pull --rebase` and tries once more. A conflict in
    /// `.ivar/skills/state.json` keeps the remote's copy and is regenerated;
    /// a conflict in a dated run file (`reports/**/YYYY-MM-DD*`,
    /// `brand/metricas/YYYY-MM-DD*`) keeps one copy when they match, or the
    /// remote's with the local one saved as `<name>-<hostname>`. A conflict in
    /// any other file, such as a memory page, a doc or anything under `brand/`,
    /// goes to the built-in `hall-sync-resolver` agent. It runs like `ivar
    /// agent run`, with the hall's default provider (the first available
    /// provider with an agent tool when the default has none), as a host run
    /// confined by Landlock to the conflicted files, with a 10-minute timeout.
    /// It is given only the files the fixed rules left, each side's version
    /// and each side's commit, and one rule: keep the content of both sides,
    /// never discard a side of a prose file. Before the commit `ivar` checks
    /// that no conflict marker (`<<<<<<<`, `=======`, `>>>>>>>`) is left, that
    /// nothing outside the conflicted files changed, and that every page under
    /// `docs/memory/` still has valid frontmatter. The commit goes through the
    /// hall's hooks, never with `--no-verify`, and carries the trailer
    /// `Resolved-by: ivar hall-sync-resolver`. `--json` lists the files in
    /// `resolved` and the human output names them, for review. If the hall's
    /// agents have a `notify` target, the first in name order is told which
    /// files were resolved; a delivery that fails is a `hall.notify_failed`
    /// warning. If a check fails, or the agent fails or runs out of time, the
    /// rebase is aborted, the hall is left as it was, and the push is a
    /// `hall.push_failed` warning. `"auto_resolve": false` in `ivar.json`
    /// turns the agent off; the fixed rules still apply and any other conflict
    /// aborts the rebase. Only the hall itself is covered, not the repos
    /// mounted in it. The agent's run does not start another sync.
    ///
    /// Commit hooks run and are never skipped. A hook that refuses, a network
    /// or authentication failure, or a missing upstream is a warning
    /// (`hall.commit_failed`, `hall.push_failed`, `hall.push_skipped`) and the
    /// commits stay local. A hall that is not a git repository, has a detached
    /// HEAD, or is in the middle of a merge, rebase or cherry-pick gets one
    /// `hall.commit_skipped` warning and nothing is committed or pushed.
    ///
    /// Run by hand it ignores `auto_commit` and `auto_push` in `ivar.json`,
    /// which only stop `ivar`'s own operations from doing this; `auto_resolve`
    /// still applies. Two runs at once do not collide: the second exits
    /// without doing anything.
    Sync,
}
