use crate::support::common;
use crate::support::graph::TestHall;

#[test]
fn e2e_feature_session_reads_its_checkout_and_refreshes_dirty_edits() {
    let hall = TestHall::new();
    hall.commit_base(
        "core",
        &[
            (
                "src/lib.rs",
                "pub fn alpha_fn() -> i32 { 10 }\npub fn keep_fn() {}\n",
            ),
            ("src/old.rs", "pub fn old_fn() {}\n"),
        ],
    );
    hall.graph_command(hall.root(), &["index", "--repo", "core"]);

    let worktree = hall.promote("rename-feat", "core");
    hall.write(
        &worktree,
        "src/lib.rs",
        "pub fn beta_fn() -> i32 { 20 }\npub fn keep_fn() {}\n",
    );
    hall.write(&worktree, "src/new.rs", "pub fn new_fn() {}\n");
    std::fs::remove_file(worktree.join("src/old.rs")).unwrap();
    hall.commit(&worktree, "rename alpha and change file set");
    let view = hall.connect_view("rename-feat");

    let feature_find = hall.graph_command(&view, &["find", "beta_fn", "--json"]);
    assert_eq!(feature_find["matches"][0]["name"], "beta_fn");
    assert!(
        hall.graph_command(&view, &["find", "alpha_fn", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        hall.graph_command(&view, &["find", "old_fn", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        hall.graph_command(&view, &["find", "new_fn", "--json"])["matches"][0]["name"],
        "new_fn"
    );

    let explored = hall.graph_command(&view, &["explore", "beta_fn", "--json"]);
    assert!(
        explored["sources"][0]["content"]
            .as_str()
            .unwrap()
            .contains("pub fn beta_fn()")
    );

    let base_find = hall.graph_command(hall.root(), &["find", "alpha_fn", "--json"]);
    assert_eq!(base_find["matches"][0]["name"], "alpha_fn");

    hall.write(
        &worktree,
        "src/lib.rs",
        "pub fn zeta_fn() -> i32 { 20 }\npub fn keep_fn() {}\n",
    );
    let dirty_find = hall.graph_command(&view, &["find", "zeta_fn", "--json"]);
    assert_eq!(dirty_find["matches"][0]["name"], "zeta_fn");
    assert!(
        hall.graph_command(&view, &["find", "beta_fn", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    hall.graph_command(
        hall.root(),
        &["clean", "--feature", "rename-feat", "--json"],
    );
    let stats = hall.graph_command(hall.root(), &["stats", "--json"]);
    assert!(
        stats["layers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|layer| layer["feature"] != "rename-feat")
    );
}

#[test]
fn e2e_feature_session_mcp_serves_uncommitted_files_from_its_checkout() {
    let hall = TestHall::new();
    hall.commit_base(
        "api",
        &[("src/access.ts", "export function evaluateAccess() {}\n")],
    );
    hall.graph_command(hall.root(), &["index", "--repo", "api"]);

    let worktree = hall.promote("rename", "api");
    hall.write(
        &worktree,
        "src/access.ts",
        "export function checkAccess() {}\n",
    );
    hall.commit(&worktree, "rename evaluateAccess");
    hall.write(
        &worktree,
        "src/session.ts",
        "export function revokeSession() {}\n",
    );
    let view = hall.connect_view("rename");

    assert_eq!(
        hall.graph_command(&view, &["find", "revokeSession", "--json"])["matches"][0]["name"],
        "revokeSession"
    );
    assert!(
        hall.graph_command(hall.root(), &["find", "revokeSession", "--json"])["matches"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    hall.write(
        &worktree,
        "src/session.ts",
        "export function endSession() {}\n",
    );
    let calls = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "refresh_index", "arguments": {}}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "graph_explore", "arguments": {"query": "endSession"}}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "graph_explore", "arguments": {"query": "checkAccess"}}}),
    ];
    let stdin: String = calls.iter().map(|call| format!("{call}\n")).collect();
    let output = common::ivar()
        .current_dir(&view)
        .args(["graph", "mcp"])
        .write_stdin(stdin)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let responses: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let text = |id: i64| {
        let response = responses.iter().find(|r| r["id"] == id).unwrap();
        assert!(response["result"]["isError"] != true, "{response}");
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    text(1);
    assert!(text(2).contains("export function endSession"));
    let check_access = text(3);
    assert!(check_access.contains("export function checkAccess"));
    assert!(!check_access.contains("evaluateAccess"));
}
