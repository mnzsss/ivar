use camino::Utf8Path;

use crate::infra::fs;

use super::super::Error;
use super::{git, run};

/// The fetch refspec every hall bare clone gets.
///
/// `git clone --bare` configures *no* `remote.origin.fetch` at all, so a bare
/// clone left as git makes it has an empty `refs/remotes/`. That is invisible
/// until something asks for a remote-tracking ref, and then it fails in a hall
/// and nowhere else:
///
/// * `git push --force-with-lease` with no explicit expectation leases against
///   `refs/remotes/origin/<branch>`; a ref that does not exist reads as
///   "stale info", and the only way out is passing the SHA by hand.
/// * `<branch>@{upstream}` does not resolve and `git status` reports no
///   ahead/behind — in a worktree a human works in every day.
///
/// What the refspec does *not* fix is a push made to a URL rather than to a
/// named remote: git records nothing local about it. That is why "already
/// pushed" is asked of the remote by [`super::remote_branch_tip`] and never of the
/// local config.
///
/// Fetching into `refs/remotes/*` rather than `refs/heads/*` is also what makes
/// the refspec safe here: git refuses to fetch into a branch that is checked
/// out in a worktree, and in a hall every branch is.
pub(crate) const REF_PREFIX_KEY: &str = "ivar.refprefix";

pub(super) fn ref_prefix(git_dir: &Utf8Path) -> String {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("config")
        .arg("--get")
        .arg(REF_PREFIX_KEY))
    .map(|s| s.trim().to_owned())
    .unwrap_or_default()
}

pub(super) fn remote_branch_ref(git_dir: &Utf8Path, branch: &str) -> String {
    format!("refs/heads/{}{branch}", ref_prefix(git_dir))
}

pub(super) fn tracking_refspec(prefix: &str) -> String {
    format!("+refs/heads/{prefix}*:refs/remotes/origin/*")
}

pub(super) const REMOTE_TRACKING_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";

/// `git clone --bare <url> <dest>`.
///
/// Both `-c` settings sit *after* `clone`, which is the persisting form: git
/// writes them into the new repository's config rather than applying them to
/// this invocation. That is deliberate for each.
///
/// The refspec has to persist — see [`REMOTE_TRACKING_REFSPEC`] for what breaks
/// without it, all of it long after the clone returned.
///
/// For GitHub HTTPS URLs the credential helper persists too, because the clone
/// is not the last time this repo talks to the remote: every later fetch and
/// every push a human makes from a worktree needs the same token. What is
/// written to `.git/config` is the *command*, never its answer — the token is
/// re-derived from `gh`/`$GITHUB_TOKEN` on each call and never lands on disk,
/// which is the property that matters and the reason a helper is registered
/// instead of a credential stored.
pub(crate) fn clone_bare(url: &str, dest: &Utf8Path) -> Result<(), Error> {
    let mut cmd = git()
        .arg("clone")
        .arg("--bare")
        .arg("-c")
        .arg(format!("remote.origin.fetch={REMOTE_TRACKING_REFSPEC}"));
    if crate::infra::github::is_github_https(url) {
        cmd = cmd.arg("-c").arg("credential.helper=!ivar git-credential");
    }
    cmd = cmd.arg(url).arg(dest.as_str());
    run(&cmd)?;
    Ok(())
}

pub(crate) fn clone_bare_prefixed(url: &str, dest: &Utf8Path, prefix: &str) -> Result<(), Error> {
    let created = !fs::exists(dest).unwrap_or(false);
    let result = (|| {
        run(&git().arg("init").arg("--bare").arg(dest.as_str()))?;
        let config = |key: &str, value: &str| {
            run(&git()
                .arg("--git-dir")
                .arg(dest.as_str())
                .arg("config")
                .arg(key)
                .arg(value))
        };
        config("remote.origin.url", url)?;
        config("remote.origin.fetch", &tracking_refspec(prefix))?;
        if !prefix.is_empty() {
            config(REF_PREFIX_KEY, prefix)?;
        }
        if crate::infra::github::is_github_https(url) {
            config("credential.helper", "!ivar git-credential")?;
        }
        run(&git()
            .arg("--git-dir")
            .arg(dest.as_str())
            .arg("fetch")
            .arg("--quiet")
            .arg("origin")
            .arg(format!("+refs/heads/{prefix}*:refs/heads/*"))
            .arg(tracking_refspec(prefix)))?;
        Ok(())
    })();

    if result.is_err() && created {
        let _ = fs::remove_path(dest);
    }
    result
}

/// Point `git_dir`'s origin at the tracking refspec, whatever it was
/// set to before.
///
/// The repair path for halls cloned by a build that did not configure it.
/// Re-cloning is not an option once feature branches live in the bare, so
/// `sync` sets it in place on every run; the refs themselves appear at the
/// next fetch.
///
/// `--replace-all` rather than plain set: a key with several values makes
/// `git config` refuse outright, and this is the one refspec the bare is
/// supposed to have — collapsing to it is the point, and the bare under
/// `.ivar/repos/` is ivar's to normalise.
pub(crate) fn ensure_remote_tracking(git_dir: &Utf8Path) -> Result<(), Error> {
    run(&git()
        .arg("--git-dir")
        .arg(git_dir.as_str())
        .arg("config")
        .arg("--replace-all")
        .arg("remote.origin.fetch")
        .arg(tracking_refspec(&ref_prefix(git_dir))))?;
    Ok(())
}
