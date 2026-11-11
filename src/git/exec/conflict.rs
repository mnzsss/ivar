use camino::{Utf8Path, Utf8PathBuf};

use super::super::{CommitInfo, Conflict, Error, Side};
use super::{git, run};

/// `git ls-files -u -z` — every path left unmerged, with the blob each side
/// holds.
///
/// Each record is `<mode> <blob> <stage>\t<path>`: stage 1 is the common
/// ancestor, 2 is *ours* and 3 is *theirs*. A side that deleted the path has no
/// record at all, which is how `None` is told apart from an empty file.
pub(crate) fn conflicts(worktree: &Utf8Path) -> Result<Vec<Conflict>, Error> {
    let stdout = run(&git()
        .cwd(worktree)
        .arg("--no-optional-locks")
        .arg("ls-files")
        .arg("--unmerged")
        .arg("-z"))?;
    let mut found: Vec<Conflict> = Vec::new();
    for record in stdout.split('\0') {
        let Some((meta, path)) = record.split_once('\t') else {
            continue;
        };
        let mut fields = meta.split_whitespace();
        let (Some(_mode), Some(blob), Some(stage)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let path = Utf8PathBuf::from(path);
        let index = if let Some(index) = found.iter().position(|entry| entry.path == path) {
            index
        } else {
            found.push(Conflict {
                path,
                ours: None,
                theirs: None,
            });
            found.len() - 1
        };
        if let Some(entry) = found.get_mut(index) {
            match stage {
                "2" => entry.ours = Some(blob.to_owned()),
                "3" => entry.theirs = Some(blob.to_owned()),
                _ => {}
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// `git checkout --ours|--theirs -- <path>`.
pub(crate) fn take_side(worktree: &Utf8Path, path: &Utf8Path, side: Side) -> Result<(), Error> {
    run(&git()
        .cwd(worktree)
        .arg("--literal-pathspecs")
        .arg("checkout")
        .arg(match side {
            Side::Ours => "--ours",
            Side::Theirs => "--theirs",
        })
        .arg("--")
        .arg(path.as_str()))?;
    Ok(())
}

/// `git cat-file blob <id>` — the text of one side of a conflict, trailing
/// whitespace trimmed and bytes that are not UTF-8 replaced: it is read, never
/// written back.
pub(crate) fn blob_text(worktree: &Utf8Path, blob: &str) -> Result<String, Error> {
    run(&git()
        .cwd(worktree)
        .arg("--no-optional-locks")
        .arg("cat-file")
        .arg("blob")
        .arg(blob))
}

/// `git log -1 --format=%H%n%s <revision>` — one commit's id and subject.
pub(crate) fn commit_info(worktree: &Utf8Path, revision: &str) -> Result<CommitInfo, Error> {
    let stdout = run(&git()
        .cwd(worktree)
        .arg("--no-optional-locks")
        .arg("log")
        .arg("-1")
        .arg("--format=%H%n%s")
        .arg(revision)
        .arg("--"))?;
    let mut lines = stdout.lines();
    Ok(CommitInfo {
        sha: lines.next().unwrap_or_default().trim().to_owned(),
        subject: lines.next().unwrap_or_default().trim().to_owned(),
    })
}
