#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn codex_bypasses_approvals_inside_the_container_sandbox() {
    assert_eq!(
        Tool::Codex.argv(Some("gpt-5"), "fix it"),
        [
            "codex",
            "exec",
            "--dangerously-bypass-approvals-and-sandbox",
            "--skip-git-repo-check",
            "-m",
            "gpt-5",
            "fix it"
        ]
    );
    assert!(Tool::Codex.env().is_empty());
}

#[test]
fn opencode_runs_with_every_permission_allowed() {
    assert_eq!(
        Tool::Opencode.argv(Some("anthropic/claude-sonnet-4"), "fix it"),
        [
            "opencode",
            "run",
            "-m",
            "anthropic/claude-sonnet-4",
            "fix it"
        ]
    );
    assert_eq!(
        Tool::Opencode.env(),
        [(
            "OPENCODE_CONFIG_CONTENT".to_owned(),
            r#"{"permission":{"*":"allow"}}"#.to_owned()
        )]
    );
    assert!(Tool::Claude.env().is_empty());
}

#[test]
fn claude_runs_print_mode_bypassing_permissions() {
    assert_eq!(
        Tool::Claude.argv(Some("sonnet"), "fix it"),
        [
            "claude",
            "-p",
            "--permission-mode",
            "bypassPermissions",
            "--model",
            "sonnet",
            "fix it"
        ]
    );
}

#[test]
fn opencode_runs_run_and_omits_the_model_flag_without_a_model() {
    assert_eq!(
        Tool::Opencode.argv(None, "fix it"),
        ["opencode", "run", "fix it"]
    );
}

#[test]
fn parses_lowercase_names_and_names_the_choices_otherwise() {
    assert_eq!("claude".parse::<Tool>().unwrap(), Tool::Claude);
    let error = "gemini".parse::<Tool>().unwrap_err();
    assert!(error.contains("codex, claude, opencode"), "{error}");
}

#[test]
fn reads_cost_from_the_tools_json_usage_line_when_present() {
    let out = "working\n{\"type\":\"result\",\"total_cost_usd\":0.0421}\n";
    assert_eq!(Tool::Claude.cost_usd(out), Some(0.0421));
    assert_eq!(Tool::Codex.cost_usd("{\"cost_usd\":1.5}\n"), Some(1.5));
    assert_eq!(Tool::Claude.cost_usd("no usage here\n{not json"), None);
    assert_eq!(Tool::Codex.cost_usd(out), None);
}

#[test]
fn detects_a_prompt_for_input_case_insensitively() {
    assert!(Tool::Claude.is_blocked("Error: This command REQUIRES APPROVAL"));
    assert!(!Tool::Claude.is_blocked("all done"));
    assert!(Tool::Codex.is_blocked("approval required for command"));
    assert!(Tool::Opencode.is_blocked("permission requested: bash"));
}

#[test]
fn credentials_name_each_tools_login_file_and_honour_its_overrides() {
    use camino::Utf8Path;
    let home = Utf8Path::new("/home/u");
    let none = |_: &str| None;
    let paths = |tool: Tool, var: &dyn Fn(&str) -> Option<String>| {
        tool.credentials(home, var)
            .into_iter()
            .map(|c| (c.real.to_string(), c.private, c.write_back))
            .collect::<Vec<_>>()
    };
    let own = |s: &str| s.to_owned();
    assert_eq!(
        paths(Tool::Claude, &none),
        [
            (
                own("/home/u/.claude/.credentials.json"),
                ".credentials.json",
                true
            ),
            (own("/home/u/.claude.json"), ".claude.json", false),
        ]
    );
    assert_eq!(
        paths(Tool::Claude, &|k| (k == "CLAUDE_CONFIG_DIR")
            .then(|| own("/srv/c"))),
        [
            (own("/srv/c/.credentials.json"), ".credentials.json", true),
            (own("/srv/c/.claude.json"), ".claude.json", false),
        ]
    );
    assert_eq!(
        paths(Tool::Codex, &|k| (k == "CODEX_HOME").then(|| own("/srv/x"))),
        [(own("/srv/x/auth.json"), "auth.json", true)]
    );
    assert_eq!(
        paths(Tool::Opencode, &none),
        [(
            own("/home/u/.local/share/opencode/auth.json"),
            "data/opencode/auth.json",
            true
        )]
    );
}

#[test]
fn state_env_points_each_tool_at_its_private_state_dir() {
    use camino::Utf8Path;
    let state = Utf8Path::new("/hall/.ivar/agents/state/a/t");
    let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
    assert_eq!(
        Tool::Claude.state_env(state),
        [pair("CLAUDE_CONFIG_DIR", "/hall/.ivar/agents/state/a/t")]
    );
    assert_eq!(
        Tool::Codex.state_env(state),
        [pair("CODEX_HOME", "/hall/.ivar/agents/state/a/t")]
    );
    assert_eq!(
        Tool::Opencode.state_env(state),
        [
            pair("XDG_DATA_HOME", "/hall/.ivar/agents/state/a/t/data"),
            pair("XDG_STATE_HOME", "/hall/.ivar/agents/state/a/t/state"),
            pair("XDG_CONFIG_HOME", "/hall/.ivar/agents/state/a/t/config"),
            pair("XDG_CACHE_HOME", "/hall/.ivar/agents/state/a/t/cache"),
        ]
    );
}
