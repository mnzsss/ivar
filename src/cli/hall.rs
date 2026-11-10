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
    /// any other file aborts the rebase and leaves the hall as it was.
    ///
    /// Commit hooks run and are never skipped. A hook that refuses, a network
    /// or authentication failure, or a missing upstream is a warning
    /// (`hall.commit_failed`, `hall.push_failed`, `hall.push_skipped`) and the
    /// commits stay local. A hall that is not a git repository, has a detached
    /// HEAD, or is in the middle of a merge, rebase or cherry-pick gets one
    /// `hall.commit_skipped` warning and nothing is committed or pushed.
    ///
    /// Run by hand it ignores `auto_commit` and `auto_push` in `ivar.json`,
    /// which only stop `ivar`'s own operations from doing this. Two runs at
    /// once do not collide: the second exits without doing anything.
    Sync,
}
