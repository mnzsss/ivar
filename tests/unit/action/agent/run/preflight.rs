use std::io::{Read, Write};
use std::net::TcpListener;

use super::preflight::{AUTH, Block, UNAVAILABLE, check, probe_tool};
use super::*;
use crate::action::agent::requirements::find_on_path;
use crate::domain::agent::spec::AgentSpec;
use crate::domain::agent::tool::Tool;

const LAUNCHER: &str = "/opt/ivar/bin/ivar";

fn deps(
    home: &Utf8Path,
    tool_check: fn(Tool, &[(String, String)], &Utf8Path) -> ToolCheck,
) -> HostDeps {
    HostDeps {
        launcher: Utf8PathBuf::from(LAUNCHER),
        home: home.to_path_buf(),
        parent_env: vec![("HOME".into(), home.to_string())],
        landlock: true,
        tool_check,
    }
}

fn spec(frontmatter: &str) -> AgentSpec {
    let (_guard, root) = hall_with_agent(frontmatter);
    // The hall goes with the guard; the spec is plain data.
    crate::store::agent::load(&Layout::at(root), "nightly").unwrap()
}

fn home_with_login(contents: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, dir) = crate::test_support::utf8_temp_dir();
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    if let Some(contents) = contents {
        std::fs::write(home.join(".claude/.credentials.json"), contents).unwrap();
    }
    (guard, home)
}

fn check_with(
    spec: &AgentSpec,
    home: &Utf8Path,
    tool_check: fn(Tool, &[(String, String)], &Utf8Path) -> ToolCheck,
) -> Result<(), Block> {
    let deps = deps(home, tool_check);
    check(spec, &|| Ok(deps.clone()), &home.join("scratch"))
}

fn ready(_: Tool, _: &[(String, String)], _: &Utf8Path) -> ToolCheck {
    ToolCheck::Ready
}

fn missing(_: Tool, _: &[(String, String)], _: &Utf8Path) -> ToolCheck {
    ToolCheck::Missing
}

fn broken(_: Tool, _: &[(String, String)], _: &Utf8Path) -> ToolCheck {
    ToolCheck::Broken("`claude --version` ended with 127: boom".into())
}

#[test]
fn a_tool_that_answers_with_a_login_passes() {
    let (_guard, home) = home_with_login(Some(r#"{"claudeAiOauth":{"accessToken":"a"}}"#));
    let spec = spec("runtime: host\ntool: claude\n");
    assert_eq!(check_with(&spec, &home, ready), Ok(()));
}

#[test]
fn a_missing_tool_is_provider_unavailable() {
    let (_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let spec = spec("runtime: host\ntool: claude\n");
    let block = check_with(&spec, &home, missing).unwrap_err();
    assert_eq!(block.code, UNAVAILABLE);
    assert_eq!(block.reason, "provider unavailable");
    assert!(
        block.detail.contains("`claude` is not installed"),
        "{block:?}"
    );
    let message = block.message("nightly");
    assert!(
        message.starts_with("agent.provider_unavailable: provider unavailable: "),
        "{message}"
    );
    assert!(
        message.ends_with("run `ivar agent resume nightly`."),
        "{message}"
    );
}

#[test]
fn a_tool_that_does_not_run_is_provider_unavailable_with_why() {
    let (_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let spec = spec("runtime: host\ntool: claude\n");
    let block = check_with(&spec, &home, broken).unwrap_err();
    assert_eq!(block.code, UNAVAILABLE);
    assert!(block.detail.contains("ended with 127: boom"), "{block:?}");
}

#[test]
fn no_login_file_is_an_auth_error_naming_the_file() {
    let (_guard, home) = home_with_login(None);
    let spec = spec("runtime: host\ntool: claude\n");
    let block = check_with(&spec, &home, ready).unwrap_err();
    assert_eq!(block.code, AUTH);
    assert_eq!(block.reason, "auth error");
    assert!(block.detail.contains("not logged in"), "{block:?}");
    assert!(
        block
            .detail
            .contains(home.join(".claude/.credentials.json").as_str()),
        "{block:?}"
    );
    assert!(block.detail.contains("ANTHROPIC_API_KEY"), "{block:?}");
}

#[test]
fn an_empty_or_unparseable_login_is_an_auth_error() {
    for contents in ["", "{}", "not json"] {
        let (_guard, home) = home_with_login(Some(contents));
        let spec = spec("runtime: host\ntool: claude\n");
        let block = check_with(&spec, &home, ready).unwrap_err();
        assert_eq!(block.code, AUTH, "{contents:?}");
        assert!(block.detail.contains("not valid JSON"), "{block:?}");
    }
}

#[test]
fn an_api_key_in_secrets_stands_in_for_a_login_file() {
    let (_guard, home) = home_with_login(None);
    let spec = spec("runtime: host\ntool: claude\nsecrets: [ANTHROPIC_API_KEY]\n");
    assert_eq!(check_with(&spec, &home, ready), Ok(()));
    let other = spec_with_secret("TOKEN");
    assert_eq!(check_with(&other, &home, ready).unwrap_err().code, AUTH);
}

fn spec_with_secret(name: &str) -> AgentSpec {
    spec(&format!("runtime: host\ntool: claude\nsecrets: [{name}]\n"))
}

#[test]
fn a_missing_tool_outranks_a_missing_login() {
    let (_guard, home) = home_with_login(None);
    let spec = spec("runtime: host\ntool: claude\n");
    assert_eq!(
        check_with(&spec, &home, missing).unwrap_err().code,
        UNAVAILABLE
    );
}

#[test]
fn docker_and_image_agents_are_not_checked() {
    let (_guard, home) = home_with_login(None);
    for frontmatter in [
        "tool: claude\n",
        "image: curlimages/curl:8.10.1\ncommand: [true]\n",
    ] {
        let spec = spec(frontmatter);
        assert_eq!(check_with(&spec, &home, missing), Ok(()), "{frontmatter}");
    }
}

#[test]
fn a_host_that_cannot_be_described_is_left_to_the_run() {
    let spec = spec("runtime: host\ntool: claude\n");
    let result = check(
        &spec,
        &|| Err(Failure::failed("agent.home_unknown", "HOME is not set")),
        Utf8Path::new("/nowhere"),
    );
    assert_eq!(result, Ok(()));
}

fn fake_tool(dir: &Utf8Path, body: &str) -> Vec<(String, String)> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("claude");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    vec![
        ("PATH".into(), dir.to_string()),
        ("HOME".into(), dir.to_string()),
    ]
}

#[test]
fn the_probe_runs_the_tool_on_the_runs_own_path() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let env = fake_tool(&dir.join("bin"), "echo 2.0.1");
    assert_eq!(
        probe_tool(Tool::Claude, &env, &dir.join("scratch")),
        ToolCheck::Ready
    );
    assert!(find_on_path("claude", std::ffi::OsStr::new(dir.join("bin").as_str())).is_some());
}

#[test]
fn the_probe_tells_a_broken_install_from_a_missing_one() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let env = fake_tool(&dir.join("bin"), "echo 'cannot load libfoo' >&2\nexit 3");
    match probe_tool(Tool::Claude, &env, &dir.join("scratch")) {
        ToolCheck::Broken(why) => {
            assert!(why.contains("ended with 3"), "{why}");
            assert!(why.contains("cannot load libfoo"), "{why}");
        }
        other => panic!("expected Broken, got {other:?}"),
    }
    let empty = vec![("PATH".to_owned(), dir.join("nothing").to_string())];
    assert_eq!(
        probe_tool(Tool::Claude, &empty, &dir.join("scratch")),
        ToolCheck::Missing
    );
    assert_eq!(
        probe_tool(Tool::Claude, &[], &dir.join("scratch")),
        ToolCheck::Missing
    );
}

// -- through a run ----------------------------------------------------------

const FAKE_LAUNCHER: &str = "printf '%s\\n' \"$*\" >> \"$HOST_FAKE_LOG\"\necho ran\nexit 0\n";

fn run_with_deps(root: &Utf8PathBuf, deps: &HostDeps) -> AgentRunReceipt {
    run_with(
        &Ctx::new(root.clone()),
        RunInput {
            name: "nightly".into(),
            trigger: Trigger::Manual,
        },
        &|| Ok(deps.clone()),
        &|_| true,
    )
    .unwrap()
    .value
}

fn launcher_log(root: &Utf8Path) -> std::path::PathBuf {
    root.parent().unwrap().join("launcher.log").into()
}

fn redirect_launcher(root: &Utf8Path) -> crate::infra::proc::Redirect {
    redirect_on_this_thread(
        LAUNCHER,
        Command::new("sh")
            .args(["-c", FAKE_LAUNCHER, "ivar"])
            .env("HOST_FAKE_LOG", launcher_log(root).to_str().unwrap()),
    )
}

fn set_problem_notify(root: &Utf8Path, value: &str) {
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest["agents"] = serde_json::json!({ "problem_notify": value });
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
}

#[test]
fn a_failed_preflight_records_a_blocked_receipt_and_launches_nothing() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let (_home_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let _r = redirect_launcher(&root);

    let receipt = run_with_deps(&root, &deps(&home, missing));

    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert!(
        receipt
            .output
            .starts_with("agent.provider_unavailable: provider unavailable:"),
        "{}",
        receipt.output
    );
    assert_eq!(receipt.exit_code, None);
    assert!(!launcher_log(&root).exists(), "the tool was launched");
    assert_eq!(stored(&root, &receipt), receipt);
    assert!(receipt.is_resumable());
    assert!(!root.join(".ivar/agents/tmp").join(&receipt.run_id).exists());
    assert_eq!(
        crate::store::agent::read_current_run(&Layout::at(root.clone()), "nightly"),
        None
    );
}

#[test]
fn a_failed_auth_preflight_says_auth_error() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let (_home_guard, home) = home_with_login(None);
    let _r = redirect_launcher(&root);

    let receipt = run_with_deps(&root, &deps(&home, ready));

    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert!(
        receipt
            .output
            .starts_with("agent.provider_auth: auth error:"),
        "{}",
        receipt.output
    );
    assert!(!launcher_log(&root).exists());
}

#[test]
fn a_passing_preflight_lets_the_run_start() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let (_home_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let _r = redirect_launcher(&root);

    let receipt = run_with_deps(&root, &deps(&home, ready));

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert!(launcher_log(&root).exists());
}

fn one_shot_server() -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/problems", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&request);
            if let Some(head_end) = text.find("\r\n\r\n") {
                let length = text[..head_end]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= head_end + 4 + length {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    (url, handle)
}

#[test]
fn a_failed_preflight_tells_the_hall_problem_notifier() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let (_home_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let _r = redirect_launcher(&root);
    let (url, server) = one_shot_server();
    set_problem_notify(&root, &url);

    let receipt = run_with_deps(&root, &deps(&home, missing));

    let request = server.join().unwrap();
    assert!(request.starts_with("POST /problems "), "{request}");
    assert!(request.contains(r#""kind": "agent_problem""#), "{request}");
    assert!(request.contains(r#""agent": "nightly""#), "{request}");
    assert!(request.contains("provider unavailable"), "{request}");
    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert_eq!(receipt.notify_error, None);
}

#[test]
fn an_unreachable_problem_notifier_leaves_the_blocked_outcome() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let (_home_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let _r = redirect_launcher(&root);
    let dead = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/problems", listener.local_addr().unwrap())
    };
    set_problem_notify(&root, &dead);

    let receipt = run_with_deps(&root, &deps(&home, missing));

    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    let error = receipt.notify_error.as_deref().unwrap();
    assert!(error.starts_with("problem notifier: "), "{error}");
    assert_eq!(stored(&root, &receipt), receipt);
}

#[test]
fn a_run_that_succeeds_never_tells_the_problem_notifier() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let (_home_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let _r = redirect_launcher(&root);
    set_problem_notify(&root, "http://127.0.0.1:1/never");

    let receipt = run_with_deps(&root, &deps(&home, ready));

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert_eq!(receipt.notify_error, None);
}

#[test]
fn a_blocked_precondition_failure_is_a_blocked_receipt() {
    // A secrets file other users can read is a precondition `ivar` refuses.
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\nsecrets: [TOKEN]\n");
    let (_home_guard, home) = home_with_login(Some(r#"{"t":1}"#));
    let _r = redirect_launcher(&root);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.join(".ivar/secrets/agents.env"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let receipt = run_with_deps(&root, &deps(&home, ready));

    assert_eq!(
        receipt.outcome,
        AgentRunOutcome::Blocked,
        "{}",
        receipt.output
    );
    assert!(receipt.is_resumable());
}
