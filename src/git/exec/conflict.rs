use camino::{Utf8Path, Utf8PathBuf};

use super::super::{Conflict, Error, Side};
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
