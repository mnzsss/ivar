#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

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
        &BTreeMap::new(),
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
            "TMP",
            "TEMP",
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
    for key in ["TMPDIR", "TMP", "TEMP"] {
        assert!(
            env.contains(&(key.to_owned(), "/hall/.ivar/agents/tmp/r1/tmp".to_owned())),
            "{key} must point at the run scratch"
        );
    }
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
        &BTreeMap::new(),
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

#[test]
fn the_agents_env_is_added_once_and_replaces_tmpdir_and_kept_names() {
    let extra: BTreeMap<String, String> = [
        ("TMPDIR", "/tmp"),
        ("LANG", "pt_BR.UTF-8"),
        ("XDG_CURRENT_DESKTOP", "KDE"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let env = host_env(
        &pairs(&[
            ("PATH", "/usr/bin"),
            ("HOME", "/home/u"),
            ("LANG", "C.UTF-8"),
        ]),
        "/t",
        &extra,
        &[],
        Tool::Claude,
        camino::Utf8Path::new("/state"),
    );
    let values = |key: &str| {
        env.iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(values("TMPDIR"), ["/tmp"]);
    assert_eq!(values("LANG"), ["pt_BR.UTF-8"]);
    assert_eq!(values("XDG_CURRENT_DESKTOP"), ["KDE"]);
    assert_eq!(values("CLAUDE_CONFIG_DIR"), ["/state"]);
}

#[test]
fn the_agents_env_can_replace_tmp_and_temp_independently() {
    let extra: BTreeMap<String, String> = [("TMP", "/var/tmp")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let env = host_env(
        &[],
        "/t",
        &extra,
        &[],
        Tool::Claude,
        camino::Utf8Path::new("/state"),
    );
    let values = |key: &str| {
        env.iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(values("TMPDIR"), ["/t"]);
    assert_eq!(values("TMP"), ["/var/tmp"]);
    assert_eq!(values("TEMP"), ["/t"]);
}
