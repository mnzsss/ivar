use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::support::graph::TestHall;
use ivar::action::Ctx;
use ivar::action::graph::input::GraphViewInput;
use ivar::action::graph::view::{ViewSeed, ViewerGraph, parse_http_request};
use ivar::action::graph::view_cmd;

const BASE_LIB: &str = "pub trait Shape {}\npub struct Circle;\nimpl Shape for Circle {}\npub fn alpha_fn() -> i32 { 10 }\npub fn keep_fn() -> i32 { alpha_fn() }\n";
const FEATURE_LIB: &str = "pub trait Shape {}\npub struct Square;\nimpl Shape for Square {}\npub fn beta_fn() -> i32 { 20 }\npub fn keep_fn() -> i32 { beta_fn() }\n";

struct RenamedHall {
    hall: TestHall,
    view: Utf8PathBuf,
}

fn renamed_hall() -> RenamedHall {
    let hall = TestHall::new();
    hall.commit_base(
        "core",
        &[
            ("src/lib.rs", BASE_LIB),
            (
                "tests/alpha_test.rs",
                "use core::alpha_fn;\n#[test]\nfn alpha_works() { alpha_fn(); }\n",
            ),
        ],
    );
    hall.graph_command(hall.root(), &["index", "--repo", "core"]);

    let worktree = hall.promote("rename-feat", "core");
    hall.write(&worktree, "src/lib.rs", FEATURE_LIB);
    hall.write(
        &worktree,
        "tests/beta_test.rs",
        "use core::beta_fn;\n#[test]\nfn beta_works() { beta_fn(); }\n",
    );
    hall.commit(&worktree, "rename alpha to beta");
    let view = hall.connect_view("rename-feat");
    hall.graph_command(&view, &["stats", "--json"]);
    RenamedHall { hall, view }
}

fn view_input(depth: usize, limit: usize) -> GraphViewInput {
    GraphViewInput {
        seed: ViewSeed::Default,
        depth,
        limit,
        no_open: true,
        port: None,
    }
}

fn initial_graph(cwd: &Utf8Path, depth: usize, limit: usize) -> ViewerGraph {
    let session = view_cmd(&Ctx::new(cwd), view_input(depth, limit))
        .unwrap()
        .value;
    let request = format!(
        "GET /api/subgraph HTTP/1.1\r\nHost: {}\r\n\r\n",
        session.url().trim_start_matches("http://")
    );
    let response = session
        .server
        .respond(&parse_http_request(request.as_bytes()).unwrap());
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    serde_json::from_slice(&response.body).unwrap()
}

fn names(graph: &ViewerGraph) -> Vec<&str> {
    graph.nodes.iter().map(|node| node.name.as_str()).collect()
}

fn assert_no_layer_repo(values: impl IntoIterator<Item = String>) {
    for repo in values {
        assert!(!repo.contains('/'), "layer pseudo-repo leaked: {repo}");
    }
}

#[test]
fn viewer_reads_the_session_layer_and_hides_layers_in_base_mode() {
    let fixture = renamed_hall();

    let session = initial_graph(&fixture.view, 1, 400);
    let session_names = names(&session);
    assert!(session_names.contains(&"beta_fn"), "{session_names:?}");
    assert!(!session_names.contains(&"alpha_fn"), "{session_names:?}");
    assert_no_layer_repo(session.nodes.iter().map(|n| n.repo.clone()));

    let base = initial_graph(fixture.hall.root(), 1, 400);
    let base_names = names(&base);
    assert_eq!(
        base_names.iter().filter(|n| **n == "alpha_fn").count(),
        1,
        "{base_names:?}"
    );
    assert!(!base_names.contains(&"beta_fn"), "{base_names:?}");
    assert_no_layer_repo(base.nodes.iter().map(|n| n.repo.clone()));
}

#[test]
fn viewer_serves_the_requested_depth_and_limit() {
    let fixture = renamed_hall();

    let graph = initial_graph(fixture.hall.root(), 2, 2);

    assert_eq!(graph.depth, 2);
    assert!(graph.nodes.len() <= 2, "{:?}", names(&graph));
}

fn affected_tests(hall: &TestHall, cwd: &Utf8Path) -> Vec<(String, String)> {
    let result = hall.graph_command(cwd, &["affected", "src/lib.rs", "--json"]);
    result["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rec| {
            (
                rec["repo"].as_str().unwrap().to_owned(),
                rec["test_file"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn affected_resolves_changed_files_through_the_session_layer() {
    let fixture = renamed_hall();

    let session = affected_tests(&fixture.hall, &fixture.view);
    assert!(
        session.contains(&("core".to_owned(), "tests/beta_test.rs".to_owned())),
        "{session:?}"
    );
    assert!(
        !session
            .iter()
            .any(|(_, file)| file == "tests/alpha_test.rs"),
        "{session:?}"
    );

    let base = affected_tests(&fixture.hall, fixture.hall.root());
    assert!(
        base.contains(&("core".to_owned(), "tests/alpha_test.rs".to_owned())),
        "{base:?}"
    );
    assert!(
        !base.iter().any(|(_, file)| file == "tests/beta_test.rs"),
        "{base:?}"
    );
    assert_no_layer_repo(base.into_iter().map(|(repo, _)| repo));
}

fn implementations(hall: &TestHall, cwd: &Utf8Path) -> Value {
    hall.graph_command(cwd, &["hierarchy", "Shape", "--json"])
}

#[test]
fn hierarchy_reads_the_session_layer_and_hides_layers_in_base_mode() {
    let fixture = renamed_hall();

    let session = implementations(&fixture.hall, &fixture.view).to_string();
    assert!(session.contains("Square"), "{session}");
    assert!(!session.contains("Circle"), "{session}");
    assert!(!session.contains("core/"), "{session}");

    let base = implementations(&fixture.hall, fixture.hall.root()).to_string();
    assert!(base.contains("Circle"), "{base}");
    assert!(!base.contains("Square"), "{base}");
    assert!(!base.contains("core/"), "{base}");
}

fn viz_html(hall: &TestHall, cwd: &Utf8Path, name: &str) -> String {
    let output = hall.root().parent().unwrap().join(name);
    hall.graph_command(cwd, &["viz", "--output", output.as_str(), "--json"]);
    std::fs::read_to_string(&output).unwrap()
}

#[test]
fn viz_reads_the_session_layer_and_hides_layers_in_base_mode() {
    let fixture = renamed_hall();

    let session = viz_html(&fixture.hall, &fixture.view, "session-viz.html");
    assert!(session.contains("beta_fn"));
    assert!(!session.contains("alpha_fn"));

    let base = viz_html(&fixture.hall, fixture.hall.root(), "base-viz.html");
    assert!(base.contains("alpha_fn"));
    assert!(!base.contains("beta_fn"));
}

#[test]
fn explore_hides_layer_symbols_in_base_mode() {
    let fixture = renamed_hall();

    let session = fixture
        .hall
        .graph_command(&fixture.view, &["explore", "beta", "--json"])
        .to_string();
    assert!(session.contains("pub fn beta_fn"), "{session}");

    let base = fixture
        .hall
        .graph_command(fixture.hall.root(), &["explore", "beta", "--json"])
        .to_string();
    assert!(!base.contains("beta_fn"), "{base}");
}
