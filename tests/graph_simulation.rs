//! End-to-end simulation integration tests for `ivar graph` using the current
//! ivar repository checkout as the source repository in an isolated temporary hall.
//!
//! # Architecture and Safety
//!
//! - **Hermetic and non-destructive**: The current ivar source checkout is never
//!   modified or directly indexed. Instead, tests spin up an isolated [`GraphHall`]
//!   with its own temporary clone and hall environment.
//! - **Sequential Single Fixture**: A single [`GraphHall::from_current_repo()`] instance
//!   runs all validation phases sequentially in one ignored lifecycle test, executing
//!   clean read-only queries and explorations first, unchanged reindexing next, and
//!   incremental worktree mutations last.
//! - **Ignored by Default for Fast Local & PR CI**: The lifecycle scenario is marked `#[ignore = "..."]`
//!   so broad `cargo test --all-features` skips it. To execute graph simulation E2E tests locally:
//!   ```bash
//!   cargo test --profile e2e --all-features --test graph_simulation -- --ignored
//!   ```

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

#[path = "support/graph.rs"]
mod graph_support;

use graph_support::GraphHall;
use predicates::prelude::predicate;

#[test]
#[ignore = "graph simulation E2E scenario (run explicitly with `cargo test --profile e2e --all-features --test graph_simulation -- --ignored`)"]
fn test_graph_simulation_e2e_lifecycle() {
    let hall = GraphHall::from_current_repo();

    // Phase 1: Bootstrap assertions and stats
    phase_bootstrap_and_stats(&hall);

    // Phase 2: Queries and exploration on clean index
    phase_queries_and_exploration(&hall);

    // Phase 3: Positional vs stdin equivalence on clean index
    phase_affected_positional_and_stdin_equivalence(&hall);

    // Phase 4: Human-readable CLI formatting
    phase_human_output_formatting(&hall);

    // Phase 5: Incremental indexing (unchanged fast-path and dirty worktree mutation)
    phase_incremental_indexing_and_dirty_worktree(&hall);
}

fn phase_bootstrap_and_stats(hall: &GraphHall) {
    // Verify database file exists after bootstrap indexing
    assert!(
        hall.db_path.is_file(),
        "memory.db should exist at {} after indexing",
        hall.db_path
    );

    let repos = hall.initial_index["repos"]
        .as_array()
        .expect("IndexBatchOutcome repos should be an array");
    assert_eq!(repos.len(), 1, "Should have indexed 1 repository");

    let ivar_repo = &repos[0];
    assert_eq!(
        ivar_repo["repo"].as_str(),
        Some("ivar"),
        "Indexed repo should be named 'ivar'"
    );

    let files_indexed = ivar_repo["files_indexed"]
        .as_u64()
        .expect("files_indexed should be integer");
    let symbols_indexed = ivar_repo["symbols_indexed"]
        .as_u64()
        .expect("symbols_indexed should be integer");
    let edges_indexed = ivar_repo["edges_indexed"]
        .as_u64()
        .expect("edges_indexed should be integer");

    assert!(
        files_indexed > 0,
        "Expected at least one file indexed, got {files_indexed}"
    );
    assert!(
        symbols_indexed > 0,
        "Expected symbols indexed, got {symbols_indexed}"
    );
    assert!(
        edges_indexed > 0,
        "Expected edges indexed, got {edges_indexed}"
    );
    assert_eq!(
        ivar_repo["skipped_up_to_date"].as_bool(),
        Some(false),
        "Initial index should not be skipped"
    );

    // Query `ivar graph stats --json`
    let stats_json = hall.run_json(&["graph", "stats"]);
    assert_eq!(
        stats_json["repo_count"].as_u64(),
        Some(1),
        "repo_count in stats should be 1"
    );
    assert_eq!(
        stats_json["file_count"].as_u64(),
        Some(files_indexed),
        "file_count in stats should match files_indexed"
    );
    assert!(
        stats_json["symbol_count"].as_u64().unwrap_or(0) >= symbols_indexed,
        "symbol_count should be at least indexed count"
    );
    assert!(
        stats_json["db_size_bytes"].as_u64().unwrap_or(0) > 0,
        "db_size_bytes should be positive"
    );
}

fn phase_queries_and_exploration(hall: &GraphHall) {
    // 1. `graph find discover_hall`
    let find_json = hall.run_json(&["graph", "find", "discover_hall"]);
    let found_symbols = find_json["symbols"]
        .as_array()
        .expect("FindOutcome symbols should be an array");
    assert!(
        !found_symbols.is_empty(),
        "Expected to find 'discover_hall' symbol in indexed ivar repo"
    );

    let first_sym = &found_symbols[0];
    assert_eq!(
        first_sym["symbol"]["name"].as_str(),
        Some("discover_hall"),
        "Symbol name must be discover_hall"
    );

    // 2. `graph file ivar <file_path>`
    let file_path = first_sym["file_path"]
        .as_str()
        .expect("file_path should be string");
    let file_json = hall.run_json(&["graph", "file", "ivar", file_path]);
    assert_eq!(
        file_json["repo"].as_str(),
        Some("ivar"),
        "File outline repo should be 'ivar'"
    );
    assert_eq!(
        file_json["file_path"].as_str(),
        Some(file_path),
        "File outline file_path should match"
    );
    let outline_symbols = file_json["symbols"]
        .as_array()
        .expect("File outline symbols should be array");
    assert!(
        !outline_symbols.is_empty(),
        "File outline should contain symbols"
    );

    // 3. `graph explore discover_hall`
    let explore_json = hall.run_json(&["graph", "explore", "discover_hall"]);
    assert_eq!(
        explore_json["query"].as_str(),
        Some("discover_hall"),
        "Explore query should match"
    );
    let primary_symbols = explore_json["primary_symbols"]
        .as_array()
        .expect("primary_symbols should be array");
    assert!(
        !primary_symbols.is_empty(),
        "Explore should return primary_symbols for discover_hall"
    );
    assert!(
        explore_json["direct_relations"].is_array(),
        "direct_relations should be array in ExploreResult JSON"
    );
    assert!(
        explore_json["entry_points"].is_array(),
        "entry_points should be array in ExploreResult JSON"
    );
    assert!(
        explore_json["transitive_consumers"].is_array(),
        "transitive_consumers should be array in ExploreResult JSON"
    );
    // 4. `graph callers discover_hall`
    let callers_json = hall.run_json(&["graph", "callers", "discover_hall"]);
    assert_eq!(
        callers_json["symbol"].as_str(),
        Some("discover_hall"),
        "Callers outcome should echo target symbol name"
    );
    let callers_list = callers_json["callers"]
        .as_array()
        .expect("CallersOutcome callers should be an array");
    assert!(
        !callers_list.is_empty(),
        "Expected at least one caller for 'discover_hall' across ivar codebase"
    );

    let caller_entry = &callers_list[0];
    let caller_sym_id = caller_entry["caller"]["id"]
        .as_i64()
        .expect("Caller symbol must have a valid numeric id");
    let caller_sym_name = caller_entry["caller"]["name"].as_str().unwrap_or("unknown");

    // 5. `graph callees <caller_sym_id>`
    let callees_json = hall.run_json(&["graph", "callees", &caller_sym_id.to_string()]);
    assert_eq!(
        callees_json["symbol_id"].as_i64(),
        Some(caller_sym_id),
        "Callees outcome should echo symbol_id"
    );
    let callees_list = callees_json["callees"]
        .as_array()
        .expect("CalleesOutcome callees should be an array");
    assert!(
        !callees_list.is_empty(),
        "Expected outgoing callees from caller '{caller_sym_name}' (id {caller_sym_id})"
    );

    // 6. `graph path <caller_sym_name> discover_hall`
    let path_json = hall.run_json(&["graph", "path", caller_sym_name, "discover_hall"]);
    assert_eq!(
        path_json["from"].as_str(),
        Some(caller_sym_name),
        "Path 'from' should match the discovered caller"
    );
    assert_eq!(
        path_json["to"].as_str(),
        Some("discover_hall"),
        "Path 'to' should match the called symbol"
    );
    let path_steps = path_json["steps"]
        .as_array()
        .expect("Path outcome steps should be an array");
    assert!(
        !path_steps.is_empty(),
        "Expected a path from caller '{caller_sym_name}' to 'discover_hall'"
    );
}

fn phase_affected_positional_and_stdin_equivalence(hall: &GraphHall) {
    let target_changed_file = "src/action/hall/init.rs";
    let affected_pos_json = hall.run_json(&["graph", "affected", target_changed_file]);
    let affected_stdin_json =
        hall.run_json_stdin(&["graph", "affected", "--stdin"], Some(target_changed_file));

    let pos_changed = affected_pos_json["changed_files"]
        .as_array()
        .expect("positional changed_files should be array");
    let stdin_changed = affected_stdin_json["changed_files"]
        .as_array()
        .expect("stdin changed_files should be array");
    assert_eq!(
        pos_changed, stdin_changed,
        "Positional and --stdin changed_files should match"
    );

    let pos_tests = affected_pos_json["affected_test_files"]
        .as_array()
        .expect("positional affected_test_files should be array");
    let stdin_tests = affected_stdin_json["affected_test_files"]
        .as_array()
        .expect("stdin affected_test_files should be array");
    assert_eq!(
        pos_tests, stdin_tests,
        "Positional and --stdin affected_test_files should match"
    );
    assert!(
        !pos_tests.is_empty(),
        "Expected affected tests for '{target_changed_file}'"
    );
}

fn phase_human_output_formatting(hall: &GraphHall) {
    hall.run_human(&["graph", "stats"])
        .stdout(predicate::str::contains("Codebase Graph Statistics:"))
        .stdout(predicate::str::contains("Repositories: 1"))
        .stdout(predicate::str::contains("Files:"));
}

fn phase_incremental_indexing_and_dirty_worktree(hall: &GraphHall) {
    // 1. Fast-path incremental re-index without changes
    let reindex_fast_json = hall.index();
    let reindex_fast_repos = reindex_fast_json["repos"]
        .as_array()
        .expect("reindex repos should be array");
    assert_eq!(reindex_fast_repos.len(), 1);
    assert_eq!(
        reindex_fast_repos[0]["skipped_up_to_date"].as_bool(),
        Some(true),
        "Unchanged reindex should be skipped as up to date"
    );
    assert_eq!(
        reindex_fast_repos[0]["files_indexed"].as_u64(),
        Some(0),
        "Unchanged reindex should index 0 files"
    );

    // 2. Append custom valid Rust functions in the isolated synced worktree
    let target_rel_path = "src/action/hall/init.rs";
    let unique_fn_name = "custom_simulated_workflow_probe_fn";
    let unique_target_name = "custom_simulated_workflow_target_fn";
    let appended_code = format!(
        "\n\npub fn {unique_target_name}() {{}}\n\npub fn {unique_fn_name}() {{\n    {unique_target_name}();\n}}\n"
    );

    hall.append_worktree_file(target_rel_path, &appended_code);

    // 3. Reindex dirty worktree: exactly 1 file should be indexed
    let dirty_reindex_json = hall.index();
    let dirty_repos = dirty_reindex_json["repos"]
        .as_array()
        .expect("dirty reindex repos should be array");
    assert_eq!(dirty_repos.len(), 1);
    assert_eq!(
        dirty_repos[0]["skipped_up_to_date"].as_bool(),
        Some(false),
        "Dirty worktree reindex should not be skipped"
    );
    assert_eq!(
        dirty_repos[0]["files_indexed"].as_u64(),
        Some(1),
        "Exactly 1 modified file should be indexed"
    );
    assert!(
        dirty_repos[0]["symbols_indexed"].as_u64().unwrap_or(0) > 0,
        "New symbols should be indexed"
    );

    // 4. Verify newly appended symbol is discoverable via `graph find`
    let find_new_sym_json = hall.run_json(&["graph", "find", unique_fn_name]);
    let new_syms = find_new_sym_json["symbols"]
        .as_array()
        .expect("Find symbols should be array");
    assert_eq!(
        new_syms.len(),
        1,
        "Expected exactly 1 symbol found for '{unique_fn_name}'"
    );
    let new_sym_record = &new_syms[0];
    assert_eq!(
        new_sym_record["symbol"]["name"].as_str(),
        Some(unique_fn_name),
        "Discovered symbol name should match"
    );
    let new_sym_id = new_sym_record["symbol"]["id"]
        .as_i64()
        .expect("New symbol must have a valid numeric id");

    // 5. Validate the newly indexed call relationship
    let new_callees_json = hall.run_json(&["graph", "callees", &new_sym_id.to_string()]);
    let new_callees = new_callees_json["callees"]
        .as_array()
        .expect("New symbol callees should be array");
    let calls_simulated_target = new_callees.iter().any(|callee| {
        callee["callee_name"].as_str() == Some(unique_target_name)
            || callee["callee_symbol"]["name"].as_str() == Some(unique_target_name)
    });
    assert!(
        calls_simulated_target,
        "Expected new function '{unique_fn_name}' to call '{unique_target_name}'"
    );
}
