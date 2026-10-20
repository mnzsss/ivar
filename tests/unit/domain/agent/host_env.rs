#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn the_host_environment_keeps_the_login_and_drops_every_git_credential() {
    let parent = pairs(&[
        ("PATH", "/usr/bin"),
        ("HOME", "/home/u"),
        ("LANG", "C.UTF-8"),
        ("LC_ALL", "C"),
        ("XDG_CONFIG_HOME", "/home/u/.config"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("CODEX_HOME", "/home/u/.codex"),
        ("TMPDIR", "/tmp"),
        ("SSH_AUTH_SOCK", "/run/ssh"),
        ("GIT_ASKPASS", "/bin/askpass"),
        ("SSH_ASKPASS", "/bin/askpass"),
        ("GH_TOKEN", "ghp"),
        ("GITHUB_TOKEN", "ghs"),
        ("GIT_SSH_COMMAND", "ssh -i key"),
        ("ANTHROPIC_API_KEY", "sk"),
    ]);
    let env = host_env(
        &parent,
        "/hall/.ivar/agents/tmp/r1/tmp",
        &pairs(&[("TOKEN", "s3")]),
        Tool::Codex,
        camino::Utf8Path::new("/state"),
    );
    let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "PATH",
            "HOME",
            "LANG",
            "LC_ALL",
            "TMPDIR",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_NOSYSTEM",
            "GIT_TERMINAL_PROMPT",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "TOKEN",
            "CODEX_HOME"
        ]
    );
    assert!(env.contains(&(
        "TMPDIR".to_owned(),
        "/hall/.ivar/agents/tmp/r1/tmp".to_owned()
    )));
    assert!(env.contains(&("GIT_CONFIG_GLOBAL".to_owned(), "/dev/null".to_owned())));
    assert!(env.contains(&("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned())));
    assert!(env.contains(&(
        "GIT_CONFIG_KEY_0".to_owned(),
        "credential.helper".to_owned()
    )));
    assert!(env.contains(&("GIT_CONFIG_VALUE_0".to_owned(), String::new())));
    assert!(env.contains(&("CODEX_HOME".to_owned(), "/state".to_owned())));
}

#[test]
fn the_tools_state_dir_overrides_the_parents() {
    let env = host_env(
        &pairs(&[("HOME", "/home/u"), ("CLAUDE_CONFIG_DIR", "/srv/claude")]),
        "/t",
        &[],
        Tool::Claude,
        camino::Utf8Path::new("/state"),
    );
    let dirs: Vec<_> = env
        .iter()
        .filter(|(k, _)| k == "CLAUDE_CONFIG_DIR")
        .collect();
    assert_eq!(
        dirs,
        [&("CLAUDE_CONFIG_DIR".to_owned(), "/state".to_owned())]
    );
}
