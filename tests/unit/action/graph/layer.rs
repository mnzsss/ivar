use camino::Utf8PathBuf;
use ivar::action::graph::layer::ensure_layer_indexed;
use ivar::store::graph::db::GraphDb;
use ivar::store::layout::Layout;
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
    let base_commit = String::from_utf8_lossy(&out.stdout).trim().to_string();

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
