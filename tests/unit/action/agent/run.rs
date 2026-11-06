#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::agent::receipt::AgentRunOutcome;
use crate::domain::agent::tool::Credential;
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

/// The memory spool is where a hook drops a prompt for the drain, and a drained
/// `User` prompt becomes a request fingerprint. A scheduled-agent run must never
/// reach it, so what it types can never look like a user's repetition. If this
/// test fails, a run can reach the spool and prompts need an origin marker.
#[test]
fn a_container_run_never_mounts_the_memory_spool_or_a_directory_holding_it() {
    let (_guard, root) = hall_with_agent("tool: claude\n");
    let spool = Layout::at(root.clone()).memory_spool_dir();
    std::fs::create_dir_all(&spool).unwrap();
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let receipt = run_manual(&root);

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    let log = fake.log();
    assert!(log.contains("claude -p"), "the run did not launch: {log}");
    let mut words = log.split_whitespace();
    while let Some(word) = words.next() {
        if word == "-v" || word == "--volume" || word == "--mount" {
            let source = words.next().unwrap_or_default();
            let source = Utf8PathBuf::from(source.split(':').next().unwrap_or_default());
            assert!(
                !spool.starts_with(&source) && !source.starts_with(&spool),
                "{source} exposes the memory spool"
            );
        }
    }
    assert!(!log.contains(spool.as_str()), "{log}");
}

#[test]
fn a_container_workspace_mounts_only_repo_worktrees() {
    let (_guard, root) = hall_with_agent("tool: claude\n");
    let layout = Layout::at(root);
    let manifest = read_manifest(&layout).unwrap();
    let spec = crate::store::agent::load(&layout, "nightly").unwrap();

    let ws = read_workspace(&layout, &manifest, &spec);

    for mount in &ws.mounts {
        let source = Utf8PathBuf::from(&mount.source);
        assert!(source.starts_with(layout.root().join(".ivar/repos")));
        assert!(!layout.memory_spool_dir().starts_with(&source));
    }
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

fn run_with_output(frontmatter: &str, output: &str) -> AgentRunReceipt {
    let (_guard, root) = hall_with_agent(frontmatter);
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake)
            .env("DOCKER_FAKE_OUTPUT", output)
            .env("DOCKER_FAKE_EXIT", "0"),
    );
    run_manual(&root)
}

#[test]
fn a_run_that_ends_with_blocked_is_recorded_blocked_with_the_tool_exit_code() {
    let receipt = run_with_output("tool: claude\n", "tried\nBLOCKED: no token");
    assert_eq!(receipt.outcome, AgentRunOutcome::Blocked);
    assert_eq!(receipt.exit_code, Some(0));
}

#[test]
fn a_run_that_ends_with_failed_is_recorded_failed() {
    let receipt = run_with_output("tool: claude\n", "tried\nFAILED: boom");
    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert_eq!(receipt.exit_code, Some(0));
}

#[test]
fn a_sentinel_before_the_last_line_leaves_the_run_succeeded() {
    let receipt = run_with_output("tool: claude\n", "BLOCKED: x\nall done");
    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
}

#[test]
fn an_image_run_ending_in_blocked_stays_succeeded() {
    let receipt = run_with_output(
        "image: curlimages/curl:8.10.1\ncommand: [true]\n",
        "BLOCKED: x",
    );
    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
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
fn seeding_copies_only_the_login_and_drops_a_stale_copy() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let state = dir.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("stale.json"), "{}").unwrap();
    std::fs::write(dir.join("auth.json"), "{\"t\":1}").unwrap();
    let creds = vec![
        Credential {
            real: dir.join("auth.json"),
            private: "auth.json",
            write_back: true,
        },
        Credential {
            real: dir.join("missing.json"),
            private: "sub/missing.json",
            write_back: false,
        },
    ];
    std::fs::create_dir_all(state.join("sub")).unwrap();
    std::fs::write(state.join("sub/missing.json"), "{\"old\":1}").unwrap();
    let seeded = seed_credentials(&state, creds).unwrap();
    assert_eq!(
        std::fs::read_to_string(state.join("auth.json")).unwrap(),
        "{\"t\":1}"
    );
    assert!(!state.join("sub/missing.json").exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(seeded.len(), 2);
}

#[test]
fn only_a_changed_valid_login_is_written_back() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let state = dir.join("state");
    let real = dir.join("auth.json");
    let seed = || {
        std::fs::write(&real, "{\"t\":1}").unwrap();
        seed_credentials(
            &state,
            vec![Credential {
                real: real.clone(),
                private: "auth.json",
                write_back: true,
            }],
        )
        .unwrap()
    };
    for invalid in ["", "{}", "not json", "[1]"] {
        let seeded = seed();
        std::fs::write(state.join("auth.json"), invalid).unwrap();
        write_back_credentials(&state, &seeded).unwrap();
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "{\"t\":1}",
            "{invalid:?}"
        );
    }
    let seeded = seed();
    std::fs::write(state.join("auth.json"), "{\"t\":2}").unwrap();
    std::fs::write(&real, "{\"t\":9}").unwrap();
    write_back_credentials(&state, &seeded).unwrap();
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "{\"t\":9}",
        "a newer login was clobbered"
    );

    let seeded = seed();
    std::fs::write(state.join("auth.json"), "{\"t\":2}").unwrap();
    write_back_credentials(&state, &seeded).unwrap();
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"t\":2}");
}

#[path = "run/host.rs"]
mod host;
