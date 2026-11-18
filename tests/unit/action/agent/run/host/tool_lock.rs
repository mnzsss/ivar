use super::*;

/// Logs the login the run was seeded with at start, and `end` when it ends, so
/// a test reads the order of runs and what each one saw.
const TIMELINE_LAUNCHER: &str = r#"echo "start $HOST_FAKE_TAG $(cat "$CLAUDE_CONFIG_DIR/.credentials.json" "$CODEX_HOME/auth.json" 2>/dev/null)" >> "$HOST_FAKE_LOG"
[ -n "$HOST_FAKE_REFRESH" ] && printf '%s' "$HOST_FAKE_REFRESH" > "$CLAUDE_CONFIG_DIR/.credentials.json"
[ -n "$HOST_FAKE_SLEEP" ] && sleep "$HOST_FAKE_SLEEP"
echo "end $HOST_FAKE_TAG" >> "$HOST_FAKE_LOG"
exit 0
"#;

fn add_agent(root: &Utf8Path, name: &str, frontmatter: &str) {
    std::fs::write(
        root.join(format!("agents/{name}.md")),
        format!("---\nschedule: \"@daily\"\n{frontmatter}---\nGo.\n"),
    )
    .unwrap();
}

fn timeline_command(fake: &HostFake, tag: &str, sleep: &str, refresh: &str) -> Command {
    Command::new("sh")
        .args(["-c", TIMELINE_LAUNCHER, "ivar"])
        .env("HOST_FAKE_LOG", fake.dir.join("timeline.log").as_str())
        .env("HOST_FAKE_TAG", tag)
        .env("HOST_FAKE_SLEEP", sleep)
        .env("HOST_FAKE_REFRESH", refresh)
}

/// Runs `name` on its own thread, since a redirect is per thread.
fn spawn_timeline_run(
    root: &Utf8Path,
    fake: &HostFake,
    name: &'static str,
    sleep: &'static str,
    refresh: &'static str,
) -> std::thread::JoinHandle<AgentRunReceipt> {
    let (root, dir, home) = (root.to_path_buf(), fake.dir.clone(), fake.home.clone());
    std::thread::spawn(move || {
        let fake = HostFake { dir, home };
        let _r = redirect_on_this_thread(LAUNCHER, timeline_command(&fake, name, sleep, refresh));
        run_named(&root, name, host_deps(&fake.home, true))
    })
}

fn timeline(fake: &HostFake) -> Vec<String> {
    fake.read("timeline.log")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn wait_for_timeline(fake: &HostFake, lines: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while timeline(fake).len() < lines {
        assert!(
            std::time::Instant::now() < deadline,
            "the run never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn host_runs_of_one_tool_go_one_at_a_time_and_the_second_reads_the_renewed_login() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    add_agent(&root, "second", "runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);

    let first = spawn_timeline_run(&root, &fake, "nightly", "1", "{\"token\":\"new\"}");
    wait_for_timeline(&fake, 1);
    let second = spawn_timeline_run(&root, &fake, "second", "", "");
    let receipts = [first.join().unwrap(), second.join().unwrap()];

    for receipt in &receipts {
        assert_eq!(
            receipt.outcome,
            AgentRunOutcome::Succeeded,
            "{}",
            receipt.output
        );
    }
    assert_eq!(
        timeline(&fake),
        [
            "start nightly {\"token\":\"old\"}",
            "end nightly",
            "start second {\"token\":\"new\"}",
            "end second",
        ]
        .map(str::to_owned)
    );
}

#[test]
fn host_runs_of_different_tools_do_not_wait_for_each_other() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    add_agent(&root, "second", "runtime: host\ntool: codex\n");
    let fake = HostFake::new(&root);

    let first = spawn_timeline_run(&root, &fake, "nightly", "2", "");
    wait_for_timeline(&fake, 1);
    let second = spawn_timeline_run(&root, &fake, "second", "", "");
    let receipts = [first.join().unwrap(), second.join().unwrap()];

    for receipt in &receipts {
        assert_eq!(
            receipt.outcome,
            AgentRunOutcome::Succeeded,
            "{}",
            receipt.output
        );
    }
    let lines = timeline(&fake);
    let position = |prefix: &str| lines.iter().position(|l| l.starts_with(prefix)).unwrap();
    assert!(
        position("end second") < position("end nightly"),
        "{lines:?}"
    );
}

#[test]
fn waiting_for_the_tool_lock_does_not_spend_the_agents_timeout() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    add_agent(
        &root,
        "second",
        "runtime: host\ntool: claude\ntimeout: 1s\n",
    );
    let fake = HostFake::new(&root);

    let first = spawn_timeline_run(&root, &fake, "nightly", "2.5", "");
    wait_for_timeline(&fake, 1);
    let second = spawn_timeline_run(&root, &fake, "second", "", "");

    assert_eq!(first.join().unwrap().outcome, AgentRunOutcome::Succeeded);
    let second = second.join().unwrap();
    assert_eq!(
        second.outcome,
        AgentRunOutcome::Succeeded,
        "{}",
        second.output
    );
}

#[test]
fn a_timed_out_host_run_releases_the_tool_lock() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\ntimeout: 1s\n");
    add_agent(&root, "second", "runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);

    let first = spawn_timeline_run(&root, &fake, "nightly", "20", "");
    assert_eq!(first.join().unwrap().outcome, AgentRunOutcome::TimedOut);
    let second = spawn_timeline_run(&root, &fake, "second", "", "");

    assert_eq!(second.join().unwrap().outcome, AgentRunOutcome::Succeeded);
}
