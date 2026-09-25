#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::action::graph::layer::ensure_layer_indexed;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use camino::Utf8PathBuf;
use tempfile::tempdir;

#[test]
fn test_ensure_layer_indexed_builds_delta_and_tombstones() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let layout = Layout::at(root.clone());
    let db = GraphDb::open_in_memory().unwrap();
    db.ensure_views_base_mode().unwrap();

    // Create a mock git repository with a committed base file
    let wt = root.join("repos/core/feat-1");
    std::fs::create_dir_all(&wt).unwrap();

    // Initialize git repo and make initial commit
    let run = |cmd: &[&str]| {
        let status = std::process::Command::new(cmd[0])
            .args(&cmd[1..])
            .current_dir(&wt)
            .status()
            .unwrap();
        assert!(status.success());
    };
    run(&["git", "init"]);
    run(&["git", "config", "user.name", "Test"]);
    run(&["git", "config", "user.email", "test@example.com"]);
    std::fs::write(wt.join("base.rs"), "pub fn base() {}\n").unwrap();
    std::fs::write(wt.join("to_delete.rs"), "pub fn to_delete() {}\n").unwrap();
    run(&["git", "add", "."]);
    run(&["git", "commit", "-m", "initial"]);

    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&wt)
        .output()
        .unwrap();
    let base_commit = String::from_utf8_lossy(&out.stdout).trim().to_owned();

    // Modify base.rs, delete to_delete.rs, add new.rs
    std::fs::write(wt.join("base.rs"), "pub fn base_modified() {}\n").unwrap();
    std::fs::remove_file(wt.join("to_delete.rs")).unwrap();
    std::fs::write(wt.join("new.rs"), "pub fn new_fn() {}\n").unwrap();

    // Run ensure_layer_indexed
    let res = ensure_layer_indexed(&db, &layout, "feat-1", "core", &wt, &base_commit).unwrap();
    assert!(!res.skipped);
    assert_eq!(res.indexed_files, 2); // base.rs and new.rs
    assert_eq!(res.tombstoned_files, 1); // to_delete.rs

    let tombstones = db.get_layer_tombstones(res.layer_id).unwrap();
    assert_eq!(tombstones, vec!["to_delete.rs"]);

    // Running again without changes must hit fingerprint cache and skip
    let res2 = ensure_layer_indexed(&db, &layout, "feat-1", "core", &wt, &base_commit).unwrap();
    assert!(res2.skipped);
}

struct Fixture {
    _tmp: tempfile::TempDir,
    layout: Layout,
    db: GraphDb,
    wt: Utf8PathBuf,
    base_commit: String,
}

impl Fixture {
    fn new(base_files: &[(&str, &str)]) -> Self {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let layout = Layout::at(root.clone());
        let db = GraphDb::open_in_memory().unwrap();
        db.ensure_views_base_mode().unwrap();
        let wt = root.join("repos/core/feat");
        std::fs::create_dir_all(&wt).unwrap();
        let fixture = Self {
            _tmp: tmp,
            layout,
            db,
            wt,
            base_commit: String::new(),
        };
        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.name", "Test"]);
        fixture.git(&["config", "user.email", "test@example.com"]);
        for (path, content) in base_files {
            fixture.write(path, content);
        }
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "initial"]);
        let base_commit = fixture.git(&["rev-parse", "HEAD"]);
        Self {
            base_commit,
            ..fixture
        }
    }

    fn git(&self, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&self.wt)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.wt.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    fn index(&self) -> crate::action::graph::layer::LayerBuildResult {
        ensure_layer_indexed(
            &self.db,
            &self.layout,
            "feat",
            "core",
            &self.wt,
            &self.base_commit,
        )
        .unwrap()
    }

    fn layer_paths(&self, layer_id: i64) -> Vec<String> {
        let mut paths: Vec<String> = self
            .db
            .get_files_for_repo(&format!("core/{layer_id}"))
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        paths.sort();
        paths
    }
}

#[test]
fn editing_one_changed_file_reindexes_only_that_file() {
    let fx = Fixture::new(&[("a.rs", "pub fn a() {}\n"), ("b.rs", "pub fn b() {}\n")]);
    fx.write("a.rs", "pub fn a2() {}\n");
    fx.write("b.rs", "pub fn b2() {}\n");
    fx.write("c.rs", "pub fn c() {}\n");
    assert_eq!(fx.index().indexed_files, 3);

    fx.write("b.rs", "pub fn b3() { let _x = 1; }\n");
    let res = fx.index();
    assert!(!res.skipped);
    assert_eq!(res.indexed_files, 1);
    assert_eq!(fx.layer_paths(res.layer_id), vec!["a.rs", "b.rs", "c.rs"]);
}

#[test]
fn same_size_edit_with_new_mtime_is_reindexed() {
    let fx = Fixture::new(&[("a.rs", "pub fn a() {}\n")]);
    fx.write("a.rs", "pub fn x() {}\n");
    fx.index();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fx.write("a.rs", "pub fn y() {}\n");
    let res = fx.index();
    assert_eq!(res.indexed_files, 1);
    let y_symbols: i64 = fx
        .db
        .conn()
        .query_row("SELECT COUNT(*) FROM symbols WHERE name = 'y'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(y_symbols, 1);
}

#[test]
fn file_reverted_to_base_leaves_the_layer() {
    let fx = Fixture::new(&[("a.rs", "pub fn a() {}\n"), ("b.rs", "pub fn b() {}\n")]);
    fx.write("a.rs", "pub fn a2() {}\n");
    fx.write("b.rs", "pub fn b2() {}\n");
    fx.index();

    fx.write("a.rs", "pub fn a() {}\n");
    let res = fx.index();
    assert_eq!(res.indexed_files, 0);
    assert_eq!(fx.layer_paths(res.layer_id), vec!["b.rs"]);
}

#[test]
fn untracked_file_created_after_a_no_change_call_is_picked_up() {
    let fx = Fixture::new(&[("a.rs", "pub fn a() {}\n")]);
    fx.write("a.rs", "pub fn a2() {}\n");
    fx.index();
    assert!(fx.index().skipped);

    fx.write("fresh.rs", "pub fn fresh() {}\n");
    let res = fx.index();
    assert_eq!(res.indexed_files, 1);
    assert_eq!(fx.layer_paths(res.layer_id), vec!["a.rs", "fresh.rs"]);
}

#[test]
fn edit_to_file_equal_to_base_is_picked_up() {
    let fx = Fixture::new(&[("a.rs", "pub fn a() {}\n"), ("b.rs", "pub fn b() {}\n")]);
    fx.write("a.rs", "pub fn a2() {}\n");
    fx.index();
    assert!(fx.index().skipped);

    fx.write("b.rs", "pub fn b2() {}\n");
    let res = fx.index();
    assert_eq!(res.indexed_files, 1);
    assert_eq!(fx.layer_paths(res.layer_id), vec!["a.rs", "b.rs"]);
}

#[test]
fn rename_and_delete_after_first_build_update_tombstones() {
    let fx = Fixture::new(&[
        ("a.rs", "pub fn a() {}\n"),
        ("old.rs", "pub fn old() {}\n"),
        ("gone.rs", "pub fn gone() {}\n"),
    ]);
    fx.write("a.rs", "pub fn a2() {}\n");
    fx.index();

    fx.git(&["mv", "old.rs", "new.rs"]);
    std::fs::remove_file(fx.wt.join("gone.rs")).unwrap();
    let res = fx.index();
    assert_eq!(res.indexed_files, 1);
    assert_eq!(res.tombstoned_files, 2);
    assert_eq!(
        fx.db.get_layer_tombstones(res.layer_id).unwrap(),
        vec!["gone.rs", "old.rs"]
    );
    assert_eq!(fx.layer_paths(res.layer_id), vec!["a.rs", "new.rs"]);

    fx.write("gone.rs", "pub fn gone() {}\n");
    let res = fx.index();
    assert_eq!(
        fx.db.get_layer_tombstones(res.layer_id).unwrap(),
        vec!["old.rs"]
    );
}

#[test]
fn tracked_file_matching_ignore_rules_stays_out_of_the_layer() {
    let fx = Fixture::new(&[("a.rs", "pub fn a() {}\n"), ("gen.rs", "pub fn gen() {}\n")]);
    fx.write(".gitignore", "gen.rs\n");
    fx.write("a.rs", "pub fn a2() {}\n");
    fx.write("gen.rs", "pub fn gen2() {}\n");
    let res = fx.index();
    assert_eq!(fx.layer_paths(res.layer_id), vec![".gitignore", "a.rs"]);

    fx.write("gen.rs", "pub fn gen3() {}\n");
    let res = fx.index();
    assert_eq!(res.indexed_files, 0);
    assert_eq!(fx.layer_paths(res.layer_id), vec![".gitignore", "a.rs"]);
}

#[test]
fn feature_layer_text_add_modify_delete_shadows_the_base() {
    use crate::store::graph::extractor::ExtractedFile;
    let fx = Fixture::new(&[
        ("docs/base.md", "base sentinel\n"),
        ("docs/deleted.md", "must disappear\n"),
    ]);
    fx.db
        .insert_repo("core", fx.wt.as_str(), "main", Some(&fx.base_commit))
        .unwrap();
    let empty = ExtractedFile::default();
    fx.db
        .index_extracted_file(
            "core",
            "docs/base.md",
            "h_base",
            1,
            50,
            "base sentinel\n",
            false,
            &empty,
        )
        .unwrap();
    fx.db
        .index_extracted_file(
            "core",
            "docs/deleted.md",
            "h_del",
            1,
            50,
            "must disappear\n",
            false,
            &empty,
        )
        .unwrap();

    // Feature changes
    fx.write("docs/base.md", "feature sentinel\n");
    fx.write("docs/added.md", "added sentinel\n");
    std::fs::remove_file(fx.wt.join("docs/deleted.md")).unwrap();

    let res = fx.index();
    assert_eq!(res.indexed_files, 2);
    assert_eq!(res.tombstoned_files, 1);
    let layer_repo = format!("core/{}", res.layer_id);
    fx.db
        .configure_session_mode(&[("core", &layer_repo)])
        .unwrap();

    assert!(
        fx.db
            .search_file_content("base sentinel", Some("core"), 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fx.db
            .search_file_content("feature sentinel", Some("core"), 10)
            .unwrap()[0]
            .path,
        "docs/base.md"
    );
    assert_eq!(
        fx.db
            .search_file_content("added sentinel", Some("core"), 10)
            .unwrap()[0]
            .path,
        "docs/added.md"
    );
    assert!(
        fx.db
            .search_file_content("must disappear", Some("core"), 10)
            .unwrap()
            .is_empty()
    );
}
