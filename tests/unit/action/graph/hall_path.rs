#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::Utf8PathBuf;

use super::*;
use crate::infra::progress::Silent;
use crate::store::graph::db::GraphDb;

fn hall_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(root.join(".ivar/repos/api/main")).unwrap();
    for (path, content) in files {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }
    (tmp, root)
}

#[test]
fn walking_a_hall_dir_lists_text_files_and_skips_ignored_dirs_and_symlinks() {
    let (_tmp, root) = hall_with(&[
        ("brand/DESIGN.md", "# design"),
        ("brand/moodboard/index.html", "<html></html>"),
        ("brand/node_modules/x.js", "nope"),
        ("brand/app.min.js", "nope"),
        ("docs/a.md", "outside"),
    ]);
    std::os::unix::fs::symlink(root.join("docs"), root.join("brand/linked")).unwrap();

    let files = walk_hall_dir(root.as_std_path(), "brand").unwrap();

    assert_eq!(files, vec!["brand/DESIGN.md", "brand/moodboard/index.html"]);
}

#[test]
fn indexing_a_hall_dir_stores_files_under_repo_hall_with_hall_relative_paths() {
    let (_tmp, root) = hall_with(&[("brand/DESIGN.md", "# design\n\nchariot mark")]);
    let db = GraphDb::open_in_memory().unwrap();

    let outcome = index_hall_path(&db, root.as_std_path(), "brand", false, &Silent).unwrap();

    assert_eq!(outcome.repo, HALL_REPO);
    assert_eq!(outcome.files_indexed, 1);
    let paths: Vec<String> = db
        .get_files_for_repo(HALL_REPO)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(paths, vec!["brand/DESIGN.md"]);
    assert!(db.get_repo(HALL_REPO).unwrap().is_some());
}

#[test]
fn reindexing_a_hall_dir_deletes_files_that_disappeared_and_leaves_other_dirs_alone() {
    let (_tmp, root) = hall_with(&[("brand/a.md", "a"), ("brand/b.md", "b"), ("docs/c.md", "c")]);
    let db = GraphDb::open_in_memory().unwrap();
    index_hall_path(&db, root.as_std_path(), "brand", false, &Silent).unwrap();
    index_hall_path(&db, root.as_std_path(), "docs", false, &Silent).unwrap();
    std::fs::remove_file(root.join("brand/b.md")).unwrap();

    let outcome = index_hall_path(&db, root.as_std_path(), "brand", false, &Silent).unwrap();

    assert_eq!(outcome.files_deleted, 1);
    let paths: Vec<String> = db
        .get_files_for_repo(HALL_REPO)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(paths, vec!["brand/a.md", "docs/c.md"]);
}

#[test]
fn a_hall_file_over_the_text_bound_is_stored_truncated() {
    let big = "x".repeat(300 * 1024);
    let (_tmp, root) = hall_with(&[("docs/big.md", big.as_str())]);
    let db = GraphDb::open_in_memory().unwrap();

    index_hall_path(&db, root.as_std_path(), "docs", false, &Silent).unwrap();

    let row = db
        .get_file(HALL_REPO, "docs/big.md")
        .unwrap()
        .expect("file row");
    assert!(row.content_truncated, "{row:?}");
}

#[test]
fn indexing_the_hall_skills_stores_their_scripts_under_hall_relative_paths() {
    let (_tmp, root) = hall_with(&[
        (".ivar/skills/ivar-metrics/SKILL.md", "# metrics"),
        (
            ".ivar/skills/ivar-metrics/scripts/lib/util.mjs",
            "export function tally() {}\n",
        ),
        (".ivar/setups/dev.sh", "echo no"),
    ]);
    let db = GraphDb::open_in_memory().unwrap();

    index_hall_path(&db, root.as_std_path(), ".ivar/skills", false, &Silent).unwrap();

    let paths: Vec<String> = db
        .get_files_for_repo(HALL_REPO)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(
        paths,
        vec![
            ".ivar/skills/ivar-metrics/SKILL.md",
            ".ivar/skills/ivar-metrics/scripts/lib/util.mjs"
        ]
    );
}
