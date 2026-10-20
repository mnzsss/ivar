#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::agent::receipt::AgentRunOutcome;
use crate::infra::proc::Command;
use crate::infra::proc::redirect_on_this_thread;
use crate::store::layout::Layout;
use crate::test_support::{FakeDocker, fake_docker_command, seeded_hall};

fn hall_with_agent(frontmatter: &str) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/nightly.md"),
        format!("---\nschedule: \"@daily\"\n{frontmatter}---\nGo.\n"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join(".ivar/secrets")).unwrap();
    std::fs::write(
        root.join(".ivar/secrets/agents.env"),
        "TOKEN=hunter2\nOTHER=nope\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join(".ivar/secrets/agents.env"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    (guard, root)
}

fn run_manual(root: &Utf8PathBuf) -> AgentRunReceipt {
    run(
        &Ctx::new(root.clone()),
        RunInput {
            name: "nightly".into(),
            trigger: Trigger::Manual,
        },
    )
    .unwrap()
    .value
}

fn stored(root: &Utf8PathBuf, receipt: &AgentRunReceipt) -> AgentRunReceipt {
    crate::store::agent::read_receipt(&Layout::at(root.clone()), "nightly", &receipt.run_id)
        .unwrap()
        .unwrap()
}

#[test]
fn a_successful_run_is_recorded_scrubbed_and_cleaned_up() {
    let (_guard, root) =
        hall_with_agent("tool: claude\nsecrets: [TOKEN]\nnetwork: [api.anthropic.com]\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env(
            "DOCKER_FAKE_OUTPUT",
            "used hunter2 ok\n{\"total_cost_usd\":0.25}",
        ),
    );

    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert_eq!(receipt.exit_code, Some(0));
    assert!(receipt.output.starts_with("used [redacted] ok\n"));
    assert_eq!(receipt.cost_usd, Some(0.25));
    assert_eq!(stored(&root, &receipt), receipt);

    let log = fake.log();
    assert!(log.contains(&format!(
        "network create --internal ivar-agent-{}-net",
        receipt.run_id
    )));
    assert!(log.contains("--network-alias proxy"));
    assert!(log.contains("claude -p --permission-mode bypassPermissions Go.\n"));
    assert!(
        !log.contains(".bare"),
        "the bare clone and its config are never mounted"
    );
    assert!(log.contains(&format!("rm -f ivar-agent-{}-proxy", receipt.run_id)));
    assert!(log.contains(&format!("network rm ivar-agent-{}-net", receipt.run_id)));
    assert!(!log.contains("hunter2"));
    assert!(!root.join(".ivar/agents/tmp").join(&receipt.run_id).exists());
}

#[test]
fn no_usage_line_leaves_cost_empty() {
    let (_guard, root) = hall_with_agent("tool: claude\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_OUTPUT", "done"),
    );
    assert_eq!(run_manual(&root).cost_usd, None);
}

#[test]
fn only_the_named_secrets_reach_the_env_file() {
    let (_guard, root) = hall_with_agent("tool: claude\nsecrets: [TOKEN]\n");
    let fake = FakeDocker::install(&root);
    let capture = root.join("captured.env");
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_COPY_ENV_FILE", capture.as_str()),
    );
    run_manual(&root);
    assert_eq!(std::fs::read_to_string(capture).unwrap(), "TOKEN=hunter2\n");
}

#[test]
fn a_prompt_for_input_is_blocked_even_when_the_tool_exits_zero() {
    let (_guard, root) = hall_with_agent("tool: claude\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake)
            .env("DOCKER_FAKE_OUTPUT", "Error: this action requires approval")
            .env("DOCKER_FAKE_EXIT", "0"),
    );
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert!(
        matches!(receipt.exit_code, Some(0) | None),
        "exit 0, or stopped by the watcher before it exited"
    );
}

#[test]
fn a_tool_that_asks_and_hangs_is_stopped_at_once_as_blocked() {
    let (_guard, root) = hall_with_agent("tool: claude\ntimeout: 30m\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake)
            .env(
                "DOCKER_FAKE_OUTPUT",
                "Waiting: this action requires approval",
            )
            .env("DOCKER_FAKE_SLEEP", "60"),
    );
    let started = std::time::Instant::now();
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(
        fake.log()
            .contains(&format!("stop -t 10 ivar-agent-{}", receipt.run_id))
    );
}

#[test]
fn an_overrun_is_stopped_and_recorded_as_timed_out() {
    let (_guard, root) = hall_with_agent("tool: claude\ntimeout: 1s\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_SLEEP", "20"),
    );
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::TimedOut);
    assert!(
        fake.log()
            .contains(&format!("stop -t 10 ivar-agent-{}", receipt.run_id))
    );
}

#[test]
fn a_held_lock_records_a_skipped_run_without_touching_docker() {
    let (_guard, root) = hall_with_agent("tool: claude\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));
    let _held =
        crate::infra::fs::try_lock_exclusive(&Layout::at(root.clone()).agent_lock("nightly"))
            .unwrap()
            .unwrap();
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Skipped);
    assert_eq!(stored(&root, &receipt), receipt);
    assert_eq!(fake.log(), "");
}

#[test]
fn a_missing_secret_is_a_failed_receipt_and_starts_no_container() {
    let (_guard, root) = hall_with_agent("tool: claude\nsecrets: [ABSENT]\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert!(receipt.output.contains("ABSENT"));
    assert_eq!(stored(&root, &receipt), receipt);
    assert_eq!(fake.log(), "");
}

#[test]
fn a_failure_message_never_carries_a_secret_value() {
    let (_guard, root) = hall_with_agent("tool: claude\nsecrets: [TOKEN]\n");
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_NETWORK_ERROR", "bad token hunter2"),
    );
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert!(receipt.output.contains("[redacted]"), "{}", receipt.output);
    assert!(!receipt.output.contains("hunter2"));
    assert!(!stored(&root, &receipt).output.contains("hunter2"));
}

#[test]
fn an_unspawnable_docker_is_a_failed_receipt() {
    let (_guard, root) = hall_with_agent("tool: claude\n");
    let _r = redirect_on_this_thread(
        "docker",
        crate::infra::proc::Command::new("/nonexistent/docker"),
    );
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert_eq!(stored(&root, &receipt), receipt);
}

#[test]
fn from_cli_parses_the_trigger_and_names_the_choices() {
    assert_eq!(
        RunInput::from_cli("a".into(), "schedule").unwrap().trigger,
        Trigger::Schedule
    );
    assert_eq!(
        RunInput::from_cli("a".into(), "cron").unwrap_err().code,
        "agent.invalid_trigger"
    );
}

#[test]
#[ignore = "needs Docker; builds images/proxy locally as ghcr.io/mnzsss/ivar-agent-proxy:<crate version>"]
fn agent_run_real_docker() {
    // Build the proxy locally under the tag the binary resolves, so nothing is pulled.
    let proxy = std::process::Command::new("docker")
        .args([
            "build",
            "-t",
            crate::domain::agent::images::PROXY_IMAGE,
            concat!(env!("CARGO_MANIFEST_DIR"), "/images/proxy"),
        ])
        .status()
        .unwrap();
    assert!(proxy.success());
    let (_guard, root) = hall_with_agent(
        "image: curlimages/curl:8.10.1\ncommand: [sh, -c, \"curl -sS -o /dev/null https://example.com && ! curl -sS -o /dev/null https://example.org\"]\nnetwork: [example.com]\n",
    );
    assert_eq!(run_manual(&root).outcome, AgentRunOutcome::Succeeded);
}

#[test]
fn a_skipped_run_still_notifies() {
    let (_guard, root) = hall_with_agent("tool: claude\nnotify: http://127.0.0.1:9/hook\n");
    let _held =
        crate::infra::fs::try_lock_exclusive(&Layout::at(root.clone()).agent_lock("nightly"))
            .unwrap()
            .unwrap();
    let receipt = run_manual(&root);
    assert_eq!(receipt.outcome, AgentRunOutcome::Skipped);
    assert!(receipt.notify_error.is_some(), "a POST was attempted");
}

#[test]
fn a_delivery_preview_error_never_carries_a_secret_value() {
    let (_guard, root) = hall_with_agent("tool: claude\nsecrets: [TOKEN]\n");
    let spec = store::agent::load(&Layout::at(root), "nightly").unwrap();
    let attempt = Attempt {
        ran: Ran {
            finished: Finished::Exited(Some(0)),
            output: String::new(),
        },
        delivered: Some(Delivered {
            diff: Some("+hunter2".into()),
            fingerprint: None,
            error: Some("push to https://x:hunter2@h failed".into()),
        }),
    };
    let receipt = completed(
        &spec,
        "r1".into(),
        Trigger::Manual,
        rfc3339_now(),
        &attempt,
        &["hunter2"],
    );
    assert_eq!(
        receipt.delivery_error.as_deref(),
        Some("push to https://x:[redacted]@h failed")
    );
    assert_eq!(receipt.diff.as_deref(), Some("+[redacted]"));
}

#[test]
fn host_workspace_links_each_mount_and_grants_only_writable_ones() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let ws = Workspace {
        mounts: vec![
            Mount {
                source: "/hall/.ivar/repos/api/main".into(),
                target: "/workspace/api".into(),
                read_only: true,
            },
            Mount {
                source: "/hall/.ivar/repos/web/agent-x".into(),
                target: "/workspace/web".into(),
                read_only: false,
            },
        ],
        workdir: "/workspace/web/src".into(),
    };
    let (cwd, writable) = host_workspace(&dir, &ws).unwrap();
    assert_eq!(cwd, dir.join("workspace/web/src"));
    assert_eq!(
        writable,
        [Utf8PathBuf::from("/hall/.ivar/repos/web/agent-x")]
    );
    assert_eq!(
        std::fs::read_link(dir.join("workspace/api")).unwrap(),
        std::path::PathBuf::from("/hall/.ivar/repos/api/main")
    );
}

#[test]
fn the_claude_config_is_seeded_once_and_never_overwritten() {
    let (_guard, home) = crate::test_support::utf8_temp_dir();
    let dir = home.join(".claude");
    std::fs::write(home.join(".claude.json"), "{\"login\":1}").unwrap();
    seed_claude_config(&home, &dir).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(".claude.json")).unwrap(),
        "{\"login\":1}"
    );

    std::fs::write(dir.join(".claude.json"), "{\"login\":2}").unwrap();
    seed_claude_config(&home, &dir).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(".claude.json")).unwrap(),
        "{\"login\":2}"
    );
}

#[test]
fn no_legacy_claude_config_seeds_nothing() {
    let (_guard, home) = crate::test_support::utf8_temp_dir();
    seed_claude_config(&home, &home.join(".claude")).unwrap();
    assert!(!home.join(".claude/.claude.json").exists());
}

const LAUNCHER: &str = "/opt/ivar/bin/ivar";

const FAKE_LAUNCHER: &str = r#"printf '%s\n' "$*" >> "$HOST_FAKE_LOG"
env > "$HOST_FAKE_ENV"
read line || echo "stdin:eof"
[ -n "$HOST_FAKE_GRANDCHILD" ] && { sleep 60 & echo $! > "$HOST_FAKE_GRANDCHILD"; }
[ -n "$HOST_FAKE_OUTPUT" ] && printf '%s\n' "$HOST_FAKE_OUTPUT"
[ -n "$HOST_FAKE_SLEEP" ] && sleep "$HOST_FAKE_SLEEP"
exit 0
"#;

fn host_deps(home: &Utf8Path, landlock: bool) -> HostDeps {
    HostDeps {
        launcher: Utf8PathBuf::from(LAUNCHER),
        home: home.to_path_buf(),
        parent_env: vec![
            ("PATH".into(), std::env::var("PATH").unwrap()),
            ("HOME".into(), home.to_string()),
            ("SSH_AUTH_SOCK".into(), "/run/user/1000/ssh-agent".into()),
            ("GH_TOKEN".into(), "ghp_leak".into()),
            ("GIT_ASKPASS".into(), "/usr/bin/askpass".into()),
            ("ANTHROPIC_API_KEY".into(), "sk-parent".into()),
        ],
        landlock,
    }
}

struct HostFake {
    dir: Utf8PathBuf,
    home: Utf8PathBuf,
}

impl HostFake {
    fn new(root: &Utf8Path) -> Self {
        let dir = root.parent().unwrap().join("fake-launcher");
        let home = root.parent().unwrap().join("home");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        Self { dir, home }
    }

    fn command(&self) -> Command {
        Command::new("sh")
            .args(["-c", FAKE_LAUNCHER, "ivar"])
            .env("HOST_FAKE_LOG", self.dir.join("launcher.log").as_str())
            .env("HOST_FAKE_ENV", self.dir.join("env.txt").as_str())
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }
}

fn run_host(root: &Utf8PathBuf, deps: HostDeps) -> AgentRunReceipt {
    run_with(
        &Ctx::new(root.clone()),
        RunInput {
            name: "nightly".into(),
            trigger: Trigger::Manual,
        },
        &|| Ok(deps.clone()),
    )
    .unwrap()
    .value
}

#[test]
fn a_host_run_launches_the_tool_in_the_sandbox_without_git_credentials() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\nsecrets: [TOKEN]\n");
    let fake = HostFake::new(&root);
    let _r = redirect_on_this_thread(
        LAUNCHER,
        fake.command().env("HOST_FAKE_OUTPUT", "used hunter2 ok"),
    );

    let receipt = run_host(&root, host_deps(&fake.home, true));
    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert!(
        receipt.output.contains("stdin:eof"),
        "stdin was not /dev/null"
    );
    assert!(receipt.output.contains("used [redacted] ok"));
    assert_eq!(stored(&root, &receipt), receipt);

    let argv = fake.read("launcher.log");
    assert!(argv.starts_with("agent sandbox --write "), "{argv}");
    assert!(argv.contains(&format!("--write {}", fake.home.join(".claude"))));
    assert!(argv.contains("--write /dev/null"));
    assert!(argv.contains("-- claude -p --permission-mode bypassPermissions Go.\n"));

    let env = fake.read("env.txt");
    assert!(env.contains("GIT_CONFIG_GLOBAL=/dev/null\n"));
    assert!(env.contains("GIT_CONFIG_NOSYSTEM=1\n"));
    assert!(env.contains("TOKEN=hunter2\n"));
    assert!(env.contains(&format!(
        "CLAUDE_CONFIG_DIR={}\n",
        fake.home.join(".claude")
    )));
    for absent in [
        "SSH_AUTH_SOCK",
        "GH_TOKEN",
        "GIT_ASKPASS",
        "ANTHROPIC_API_KEY",
        "sk-parent",
        "OTHER=nope",
    ] {
        assert!(!env.contains(absent), "{absent} reached the host run");
    }
    assert!(!root.join(".ivar/agents/tmp").join(&receipt.run_id).exists());
}

#[test]
fn a_blocked_host_run_kills_the_tools_whole_process_group() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let pidfile = fake.dir.join("grandchild.pid");
    let _r = redirect_on_this_thread(
        LAUNCHER,
        fake.command()
            .env("HOST_FAKE_GRANDCHILD", pidfile.as_str())
            .env("HOST_FAKE_OUTPUT", "Waiting: this action requires approval")
            .env("HOST_FAKE_SLEEP", "60"),
    );
    let started = std::time::Instant::now();
    let receipt = run_host(&root, host_deps(&fake.home, true));
    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let pid = std::fs::read_to_string(&pidfile).unwrap();
    assert!(
        crate::test_support::process_is_gone(pid.trim()),
        "grandchild {pid} survived"
    );
}

#[test]
fn an_overrunning_host_run_is_timed_out() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\ntimeout: 1s\n");
    let fake = HostFake::new(&root);
    let _r = redirect_on_this_thread(LAUNCHER, fake.command().env("HOST_FAKE_SLEEP", "20"));
    let started = std::time::Instant::now();
    let receipt = run_host(&root, host_deps(&fake.home, true));
    assert_eq!(receipt.outcome, AgentRunOutcome::TimedOut);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn a_host_run_without_landlock_is_a_failed_receipt_and_launches_nothing() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let _r = redirect_on_this_thread(LAUNCHER, fake.command());
    let receipt = run_host(&root, host_deps(&fake.home, false));
    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert!(receipt.output.contains("Landlock"), "{}", receipt.output);
    assert_eq!(fake.read("launcher.log"), "");
}

#[test]
fn a_host_run_without_landlock_fails_with_the_landlock_code() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let spec = crate::store::agent::load(&Layout::at(root.clone()), "nightly").unwrap();
    let ws = Workspace {
        mounts: Vec::new(),
        workdir: "/workspace".into(),
    };
    let Err(failure) = execute_host(
        &spec,
        &root.join("scratch"),
        &[],
        &ws,
        &host_deps(&root, false),
    ) else {
        panic!("a host run without Landlock must fail");
    };
    assert_eq!(failure.code, "agent.landlock_unavailable");
}

#[test]
fn host_workspace_refuses_a_mount_outside_workspace_or_with_a_path_name() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    for target in [
        "/elsewhere/api",
        "/workspace/a/b",
        "/workspace/..",
        "/workspace/",
    ] {
        let ws = Workspace {
            mounts: vec![Mount {
                source: "/hall/.ivar/repos/api/main".into(),
                target: target.into(),
                read_only: true,
            }],
            workdir: "/workspace".into(),
        };
        assert!(host_workspace(&dir, &ws).is_err(), "{target} accepted");
    }
}

#[test]
fn state_roots_are_created_and_never_cover_home() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let missing = home.join(".codex");
    assert_eq!(
        prepare_state_roots(&home, vec![missing.clone()]).unwrap(),
        [missing.clone()]
    );
    assert!(missing.is_dir());
    for unsafe_root in [home.clone(), dir.clone(), home.join("..")] {
        let failure = prepare_state_roots(&home, vec![unsafe_root.clone()]).unwrap_err();
        assert_eq!(
            failure.code, "agent.state_root_covers_home",
            "{unsafe_root}"
        );
    }
}
