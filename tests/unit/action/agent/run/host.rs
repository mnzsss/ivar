use rstest::rstest;

use super::*;
use crate::domain::agent::spec::WritePath;
use crate::domain::agent::tool::Tool;

const LAUNCHER: &str = "/opt/ivar/bin/ivar";

const FAKE_LAUNCHER: &str = r#"printf '%s\n' "$*" >> "$HOST_FAKE_LOG"
env > "$HOST_FAKE_ENV"
read line || echo "stdin:eof"
[ -n "$HOST_FAKE_GRANDCHILD" ] && { sleep 60 & echo $! > "$HOST_FAKE_GRANDCHILD"; }
[ -n "$HOST_FAKE_REFRESH" ] && { printf '%s' "$HOST_FAKE_REFRESH" > "$CLAUDE_CONFIG_DIR/.credentials.json"; echo '{"hooks":1}' > "$CLAUDE_CONFIG_DIR/settings.json"; }
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
        std::fs::write(
            home.join(".claude/.credentials.json"),
            "{\"token\":\"old\"}",
        )
        .unwrap();
        std::fs::write(home.join(".claude.json"), "{\"oauthAccount\":1}").unwrap();
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
        fake.command()
            .env("HOST_FAKE_OUTPUT", "used hunter2 ok")
            .env("HOST_FAKE_REFRESH", "{\"token\":\"new\"}"),
    );
    let state = root.join(".ivar/agents/state/nightly/claude");

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
    assert!(argv.contains(&format!("--write {state} ")), "{argv}");
    assert!(
        !argv.contains(fake.home.as_str()),
        "the real home is writable: {argv}"
    );
    assert_eq!(
        std::fs::read_to_string(state.join(".claude.json")).unwrap(),
        "{\"oauthAccount\":1}"
    );
    assert_eq!(
        std::fs::read_to_string(fake.home.join(".claude/.credentials.json")).unwrap(),
        "{\"token\":\"new\"}"
    );
    assert!(!fake.home.join(".claude/settings.json").exists());
    assert!(argv.contains("--write /dev/null"));
    assert!(argv.contains("-- claude -p --permission-mode bypassPermissions Go.\n"));

    let env = fake.read("env.txt");
    assert!(env.contains("GIT_CONFIG_GLOBAL=/dev/null\n"));
    assert!(env.contains("GIT_CONFIG_NOSYSTEM=1\n"));
    assert!(env.contains("TOKEN=hunter2\n"));
    assert!(env.contains(&format!("CLAUDE_CONFIG_DIR={state}\n")));
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
        &root,
        &root.join("scratch"),
        &root.join("state"),
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
        "/workspace/.",
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
        assert!(!dir.join("workspace").exists(), "{target} created the root");
    }
}

#[test]
fn a_host_run_grants_each_write_path() {
    let (_guard, root) = hall_with_agent(
        "runtime: host\ntool: claude\nwrite: [brand/metricas, \"~/.cache/metrics\"]\n",
    );
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(root.join("brand/metricas")).unwrap();
    std::fs::create_dir_all(fake.home.join(".cache/metrics")).unwrap();
    let _r = redirect_on_this_thread(LAUNCHER, fake.command());

    let receipt = run_host(&root, host_deps(&fake.home, true));
    assert_eq!(
        receipt.outcome,
        AgentRunOutcome::Succeeded,
        "{}",
        receipt.output
    );
    let argv = fake.read("launcher.log");
    for dir in [
        root.join("brand/metricas"),
        fake.home.join(".cache/metrics"),
    ] {
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert!(
            argv.contains(&format!("--write {} ", canonical.display())),
            "{argv}"
        );
    }
}

#[test]
fn a_missing_write_path_fails_the_run_before_launching() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\nwrite: [brand/metricas]\n");
    let fake = HostFake::new(&root);
    let _r = redirect_on_this_thread(LAUNCHER, fake.command());

    let receipt = run_host(&root, host_deps(&fake.home, true));
    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert!(
        receipt.output.contains("brand/metricas"),
        "{}",
        receipt.output
    );
    assert_eq!(fake.read("launcher.log"), "");
}

#[test]
fn write_roots_names_a_missing_path_with_its_code() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let failure = write_roots(&[WritePath::Hall("nope".into())], &fake.home, &root, &[])
        .expect_err("nope does not exist");
    assert_eq!(failure.code, "agent.write_missing");
}

#[rstest]
#[case::filesystem_root("/")]
#[case::home("~")]
#[case::hall_root(".")]
#[case::hall_state(".ivar/secrets")]
#[case::hall_git(".git")]
#[case::agent_files("agents")]
#[case::tool_login_dir("~/.claude")]
#[case::inside_tool_login_dir("~/.claude/hooks")]
#[case::hall_env(".env")]
#[case::hall_mcp_config(".mcp.json")]
#[case::provider_hook_config_dir(".claude")]
fn write_roots_refuse_what_would_expose_the_hall_home_or_login(#[case] raw: &str) {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    for dir in [".ivar/secrets", ".git", "agents"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    std::fs::write(root.join(".env"), "").unwrap();
    std::fs::write(root.join(".mcp.json"), "{}").unwrap();
    std::fs::create_dir_all(fake.home.join(".claude/hooks")).unwrap();
    let credentials = Tool::Claude.credentials(&fake.home, &|_| None);

    let failure = write_roots(
        &[WritePath::parse(raw).unwrap()],
        &fake.home,
        &root,
        &credentials,
    )
    .expect_err(raw);
    assert_eq!(failure.code, "agent.write_refused", "{}", failure.what);
}

#[test]
fn write_roots_allow_a_sibling_sharing_the_hall_name_prefix() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let sibling = Utf8PathBuf::from(format!("{root}-out"));
    std::fs::create_dir_all(&sibling).unwrap();

    let roots = write_roots(
        &[WritePath::Absolute(sibling.clone())],
        &fake.home,
        &root,
        &[],
    )
    .unwrap();
    assert_eq!(roots, vec![sibling]);
}

#[test]
fn write_roots_grant_hall_and_home_paths_when_the_hall_lives_in_the_home() {
    let (_guard, tmp) = crate::test_support::canonical_temp_dir();
    let home = tmp.join("home");
    let hall = home.join("personal/hall");
    std::fs::create_dir_all(hall.join("brand/metricas")).unwrap();
    std::fs::create_dir_all(home.join(".cache/ivar-metrics")).unwrap();
    let credentials = Tool::Claude.credentials(&home, &|_| None);

    let roots = write_roots(
        &[
            WritePath::Hall("brand/metricas".into()),
            WritePath::Home(".cache/ivar-metrics".into()),
        ],
        &home,
        &hall,
        &credentials,
    )
    .unwrap();
    assert_eq!(
        roots,
        vec![
            hall.join("brand/metricas"),
            home.join(".cache/ivar-metrics")
        ]
    );
}

#[cfg(unix)]
#[test]
fn write_roots_refuse_a_symlink_into_the_hall_state() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(root.join(".ivar/secrets")).unwrap();
    std::os::unix::fs::symlink(root.join(".ivar/secrets"), fake.home.join("out")).unwrap();

    let failure = write_roots(&[WritePath::Home("out".into())], &fake.home, &root, &[])
        .expect_err("the link resolves into .ivar");
    assert_eq!(failure.code, "agent.write_refused");
}

#[test]
fn a_login_dir_that_holds_the_home_refuses_write_with_its_variable() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(fake.home.join(".cache/m")).unwrap();
    let home = fake.home.to_string();
    let credentials = Tool::Claude.credentials(&fake.home, &|key| {
        (key == "CLAUDE_CONFIG_DIR").then(|| home.clone())
    });

    let failure = write_roots(
        &[WritePath::Home(".cache/m".into())],
        &fake.home,
        &root,
        &credentials,
    )
    .expect_err("tool config would sit directly in the home");
    assert_eq!(failure.code, "agent.write_refused");
    assert!(
        failure.what.contains("CLAUDE_CONFIG_DIR"),
        "{}",
        failure.what
    );
}

#[test]
fn an_agent_without_write_never_checks_protected_paths() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let home = fake.home.to_string();
    let credentials = Tool::Claude.credentials(&fake.home, &|key| {
        (key == "CLAUDE_CONFIG_DIR").then(|| home.clone())
    });
    assert!(
        write_roots(&[], &fake.home, &root, &credentials)
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn an_unresolvable_protected_path_is_named_in_the_refusal() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(root.join("brand")).unwrap();
    let _ = std::fs::remove_file(root.join(".mcp.json"));
    std::os::unix::fs::symlink("missing/../elsewhere", root.join(".mcp.json")).unwrap();

    let failure = write_roots(&[WritePath::Hall("brand".into())], &fake.home, &root, &[])
        .expect_err("an unresolvable protected path fails closed");
    assert_eq!(failure.code, "agent.write_refused");
    assert!(failure.what.contains(".mcp.json"), "{}", failure.what);
    assert!(
        failure.what.contains("does not resolve"),
        "{}",
        failure.what
    );
}

#[test]
fn a_host_run_passes_the_agents_env_to_the_tool() {
    let (_guard, root) = hall_with_agent(
        "runtime: host\ntool: claude\nenv:\n  XDG_CURRENT_DESKTOP: KDE\n  TMPDIR: /tmp\n",
    );
    let fake = HostFake::new(&root);
    let _r = redirect_on_this_thread(LAUNCHER, fake.command());

    let receipt = run_host(&root, host_deps(&fake.home, true));
    assert_eq!(
        receipt.outcome,
        AgentRunOutcome::Succeeded,
        "{}",
        receipt.output
    );
    let env = fake.read("env.txt");
    assert!(env.lines().any(|l| l == "XDG_CURRENT_DESKTOP=KDE"), "{env}");
    assert!(env.lines().any(|l| l == "TMPDIR=/tmp"), "{env}");
}
