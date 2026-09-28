use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use camino::Utf8Path;
use serde_json::{Value, json};

use crate::support::graph::TestHall;

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Mcp {
    fn spawn(cwd: &Utf8Path) -> Self {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("ivar"))
            .args(["graph", "mcp"])
            .current_dir(cwd)
            .env_remove("IVAR_SESSION_ID")
            .env_remove("IVAR_FEATURE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }
    fn call(&mut self, id: i64, query: &str) -> String {
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "graph_explore",
                "arguments": {
                    "query": query
                }
            }
        });
        writeln!(self.stdin.as_mut().unwrap(), "{req}").unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let resp: Value = serde_json::from_str(&line).unwrap();
        assert!(resp["result"]["isError"] != true, "{resp}");
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn pid(&self) -> u32 {
        self.child.id()
    }
    fn close(mut self) {
        drop(self.stdin.take());
        self.child.wait().unwrap();
    }
}

fn lease_pid(hall: &TestHall) -> String {
    std::fs::read_to_string(hall.root().join(".ivar/graph-watch.lock"))
        .unwrap()
        .trim()
        .to_owned()
}

fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn found(hall: &TestHall, name: &str) -> bool {
    hall.graph_command(hall.root(), &["find", name, "--json"])["matches"]
        .as_array()
        .is_some_and(|m| !m.is_empty())
}

#[test]
fn a_running_mcp_server_indexes_a_base_commit_without_sync() {
    let hall = TestHall::new();
    hall.commit_base("api", &[("src/lib.ts", "export function alphaFn() {}\n")]);
    hall.graph_command(hall.root(), &["index", "--repo", "api"]);
    let mut server = Mcp::spawn(hall.root());
    server.call(1, "alphaFn");

    let base = hall.root().join(".ivar/repos/api/main");
    hall.write(&base, "src/lib.ts", "export function betaFn() {}\n");
    hall.commit(&base, "rename alphaFn");

    eventually("betaFn indexed by the leader", || found(&hall, "betaFn"));
    assert!(!found(&hall, "alphaFn"));
    server.close();
}

#[test]
fn a_second_server_takes_over_when_the_leader_exits() {
    let hall = TestHall::new();
    hall.commit_base("api", &[("src/lib.ts", "export function alphaFn() {}\n")]);
    hall.graph_command(hall.root(), &["index", "--repo", "api"]);

    let mut leader = Mcp::spawn(hall.root());
    leader.call(1, "alphaFn");
    let mut follower = Mcp::spawn(hall.root());
    follower.call(1, "alphaFn");
    assert_eq!(lease_pid(&hall), leader.pid().to_string());

    leader.close();
    follower.call(2, "alphaFn");
    assert_eq!(lease_pid(&hall), follower.pid().to_string());

    let base = hall.root().join(".ivar/repos/api/main");
    hall.write(&base, "src/lib.ts", "export function gammaFn() {}\n");
    hall.commit(&base, "rename to gammaFn");
    eventually("gammaFn indexed by the new leader", || {
        found(&hall, "gammaFn")
    });
    follower.close();
}
