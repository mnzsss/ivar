#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::fs;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

use crate::infra::progress::{Progress, Silent};

#[derive(Debug, Default)]
struct Recording {
    steps: Mutex<Vec<String>>,
    clears: AtomicUsize,
}

impl Recording {
    fn steps(&self) -> Vec<String> {
        self.steps.lock().unwrap().clone()
    }

    fn clears(&self) -> usize {
        self.clears.load(Ordering::Relaxed)
    }
}

impl Progress for Recording {
    fn step(&self, message: &str) {
        self.steps.lock().unwrap().push(message.to_owned());
    }

    fn clear(&self) {
        self.clears.fetch_add(1, Ordering::Relaxed);
    }
}
fn create_git_commit(repo: &git2::Repository, message: &str) -> Result<git2::Oid, git2::Error> {
    let mut index = repo.index()?;
    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let sig = git2::Signature::now("Ivar Test", "test@ivar.run")?;
    let parent = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .and_then(|oid| repo.find_commit(oid).ok());
    let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();

    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
}

fn long_ago() -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)
}

fn set_mtime(file: &Path, mtime: std::time::SystemTime) {
    fs::File::options()
        .write(true)
        .open(file)
        .expect("open file")
        .set_modified(mtime)
        .expect("set mtime");
}

#[path = "index/incremental.rs"]
mod incremental;
#[path = "index/rebuild.rs"]
mod rebuild;
