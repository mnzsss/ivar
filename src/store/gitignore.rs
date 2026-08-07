//! The hall's `.gitignore`, kept correct without clobbering the user's.
//!
//! [`Layout::gitignore_lines`] says *which* lines a hall needs and why the
//! `.ivar/*` form is the only one that works. This module is the other half:
//! getting them into the file, on a file the user also writes in.
//!
//! # Append, never overwrite
//!
//! A hall is very often initialised inside a directory that already has a
//! `.gitignore` — `node_modules/`, build output, editor droppings. Writing the
//! hall's lines over that would be the same silent-overwrite bug `init` refuses
//! to commit against `ivar.json`, on a file people notice later. So: read what
//! is there, add only the lines that are missing, keep everything else byte for
//! byte.
//!
//! # Why both `init` and `sync` call this
//!
//! `init` writes it because a hall needs it from the first commit. `sync`
//! writes it because the hall a teammate clones has the `.gitignore` committed
//! *and* because a hall whose `.gitignore` was edited by hand is exactly the
//! case where `git pull && ivar sync` should be self-healing. Two callers, one
//! implementation — the alternative was two copies of the append rules, which
//! is how they drift.

use crate::infra::fs;
use crate::store::layout::Layout;

/// Ensure `<hall>/.gitignore` contains every line in
/// [`Layout::gitignore_lines`], adding only the ones that are missing.
///
/// Reports whether the file changed, so a caller building a sync report can say
/// "unchanged" honestly rather than claiming work it did not do.
pub fn ensure(layout: &Layout) -> Result<Changed, fs::Error> {
    let path = layout.gitignore_path();
    let original = fs::read_text(&path)?;
    let mut content = original.clone().unwrap_or_default();

    for line in Layout::gitignore_lines() {
        if content.lines().any(|existing| existing == line) {
            continue;
        }
        // A file that does not end in a newline would otherwise get the new
        // line glued onto its last one, silently turning two patterns into one
        // nonsense pattern.
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(line);
        content.push('\n');
    }

    if original.as_deref() == Some(content.as_str()) {
        return Ok(Changed::No);
    }

    fs::write_atomic(&path, content.as_bytes())?;
    Ok(if original.is_none() {
        Changed::Created
    } else {
        Changed::Yes
    })
}

/// What [`ensure`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Changed {
    /// There was no `.gitignore`; there is now.
    Created,
    /// It existed and was missing at least one line.
    Yes,
    /// It already said everything it needed to.
    No,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::test_support::utf8_temp_dir;

    fn read(layout: &Layout) -> String {
        fs::read_text(&layout.gitignore_path()).unwrap().unwrap()
    }

    #[test]
    fn an_absent_gitignore_is_created_with_exactly_the_halls_lines() {
        let (_guard, root) = utf8_temp_dir();
        let layout = Layout::at(root);

        assert_eq!(ensure(&layout).unwrap(), Changed::Created);

        assert_eq!(read(&layout), ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n");
    }

    #[test]
    fn an_existing_gitignore_keeps_its_content_and_gains_the_missing_lines() {
        let (_guard, root) = utf8_temp_dir();
        let layout = Layout::at(root);
        fs::write_text(&layout.gitignore_path(), "node_modules/\ndist/\n").unwrap();

        assert_eq!(ensure(&layout).unwrap(), Changed::Yes);

        let content = read(&layout);
        assert!(content.starts_with("node_modules/\ndist/\n"));
        assert!(content.contains(".ivar/*"));
    }

    /// `sync` runs after every `git pull`. Reporting "changed" — or worse,
    /// rewriting the file — when nothing was missing would put a spurious
    /// modification in `git status` on every run.
    #[test]
    fn a_second_call_changes_nothing_and_says_so() {
        let (_guard, root) = utf8_temp_dir();
        let layout = Layout::at(root);
        ensure(&layout).unwrap();
        let after_first = fs::read_bytes(&layout.gitignore_path()).unwrap().unwrap();

        assert_eq!(ensure(&layout).unwrap(), Changed::No);

        assert_eq!(
            fs::read_bytes(&layout.gitignore_path()).unwrap().unwrap(),
            after_first
        );
    }

    /// Without the guard, `.ivar/*` would be glued onto `node_modules/`,
    /// producing one pattern that ignores neither.
    #[test]
    fn a_file_with_no_trailing_newline_does_not_get_its_last_line_glued() {
        let (_guard, root) = utf8_temp_dir();
        let layout = Layout::at(root);
        fs::write_text(&layout.gitignore_path(), "node_modules/").unwrap();

        ensure(&layout).unwrap();

        let content = read(&layout);
        assert!(
            content.lines().any(|line| line == "node_modules/"),
            "was: {content:?}"
        );
        assert!(content.lines().any(|line| line == ".ivar/*"));
    }

    #[test]
    fn only_the_missing_lines_are_added() {
        let (_guard, root) = utf8_temp_dir();
        let layout = Layout::at(root);
        fs::write_text(&layout.gitignore_path(), ".ivar/*\n").unwrap();

        assert_eq!(ensure(&layout).unwrap(), Changed::Yes);

        let content = read(&layout);
        assert_eq!(content.matches(".ivar/*").count(), 1);
        assert!(content.contains("!.ivar/skills/"));
        assert!(content.contains("!.ivar/setups/"));
    }
}
