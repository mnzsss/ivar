// tests/unit/providers/mod.rs
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::provider::Provider;
use crate::providers::{self, Capabilities};

#[test]
fn launch_contract_returns_correct_binary_and_capabilities_for_all_providers() {
    let claude = providers::launch_contract(Provider::ClaudeCode);
    assert_eq!(claude.binary, "claude");
    assert_eq!(
        claude.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: true,
            interactive: true,
        }
    );

    let opencode = providers::launch_contract(Provider::OpenCode);
    assert_eq!(opencode.binary, "opencode");
    assert_eq!(
        opencode.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: false,
            interactive: true,
        }
    );

    let omp = providers::launch_contract(Provider::Omp);
    assert_eq!(omp.binary, "omp");
    assert_eq!(
        omp.capabilities,
        Capabilities {
            supports_resume: true,
            supports_review: false,
            interactive: true,
        }
    );
}

#[test]
fn claude_code_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::ClaudeCode, false, &[]).unwrap();
    assert!(fresh.display().starts_with("claude"));

    let resumed = providers::start_command(Provider::ClaudeCode, true, &[]).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("claude"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn opencode_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::OpenCode, false, &[]).unwrap();
    assert_eq!(fresh.display(), "opencode");

    let resumed = providers::start_command(Provider::OpenCode, true, &[]).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("opencode"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

#[test]
fn omp_fresh_start_and_resume_commands() {
    let fresh = providers::start_command(Provider::Omp, false, &[]).unwrap();
    assert_eq!(fresh.display(), "omp");

    let resumed = providers::start_command(Provider::Omp, true, &[]).unwrap();
    let display = resumed.display();
    assert!(display.starts_with("omp"), "was: {display}");
    assert!(display.contains("--continue"), "was: {display}");
}

// -- MCP allowlist (`--settings`) -----------------------------------------

/// The `enabledMcpjsonServers` value carried by the command's `--settings`
/// argument — the set of MCP servers Claude will treat as pre-approved.
fn approved_servers(command: &crate::infra::proc::Command) -> serde_json::Value {
    let args = command.arguments();
    let flag = args
        .iter()
        .position(|a| a == "--settings")
        .expect("claude argv must carry --settings");
    let raw = args.get(flag + 1).expect("--settings must carry a value");
    let settings: serde_json::Value =
        serde_json::from_str(raw).expect("--settings value must be valid JSON");
    settings
        .get("enabledMcpjsonServers")
        .cloned()
        .expect("--settings must carry enabledMcpjsonServers")
}

/// `R-CLAUDE-ALLOWLIST`: a fresh Claude start approves exactly the
/// hall-qualified servers it is given, and nothing else.
#[test]
fn claude_fresh_start_carries_the_allowlist_in_settings() {
    let allowlist = vec!["acme-figma".to_owned(), "acme-github".to_owned()];
    let command = providers::start_command(Provider::ClaudeCode, false, &allowlist).unwrap();

    assert_eq!(
        approved_servers(&command),
        serde_json::json!(["acme-figma", "acme-github"])
    );
}

/// The allowlist survives a resume: `--continue` and `--settings` are not
/// alternatives, so a resumed session approves the same servers.
#[test]
fn claude_resume_carries_continue_and_the_allowlist() {
    let allowlist = vec!["acme-figma".to_owned()];
    let command = providers::start_command(Provider::ClaudeCode, true, &allowlist).unwrap();

    assert!(command.arguments().iter().any(|a| a == "--continue"));
    assert_eq!(
        approved_servers(&command),
        serde_json::json!(["acme-figma"])
    );
}

/// `R-CLAUDE-EMPTY`: a hall declaring no MCP servers still passes the flag,
/// with an empty list. Omitting it would let Claude fall back to prompting
/// for project servers Ivar never declared.
#[test]
fn claude_empty_allowlist_still_passes_an_explicit_empty_list() {
    let command = providers::start_command(Provider::ClaudeCode, false, &[]).unwrap();

    assert_eq!(approved_servers(&command), serde_json::json!([]));
}

/// The allowlist is passed through as given: ordering is the caller's
/// responsibility, so the harness never silently reorders an approval set.
#[test]
fn claude_allowlist_is_passed_through_in_the_order_given() {
    let allowlist = vec![
        "acme-zeta".to_owned(),
        "acme-alpha".to_owned(),
        "acme-figma".to_owned(),
    ];
    let command = providers::start_command(Provider::ClaudeCode, false, &allowlist).unwrap();

    assert_eq!(
        approved_servers(&command),
        serde_json::json!(["acme-zeta", "acme-alpha", "acme-figma"])
    );
}

/// OpenCode and OMP argv are byte-identical regardless of the allowlist:
/// `--settings` is Claude's flag alone.
#[test]
fn other_providers_never_receive_settings() {
    let allowlist = vec!["acme-figma".to_owned()];
    for provider in [Provider::OpenCode, Provider::Omp] {
        for resume in [false, true] {
            let with = providers::start_command(provider, resume, &allowlist).unwrap();
            let without = providers::start_command(provider, resume, &[]).unwrap();
            assert_eq!(with.display(), without.display());
            assert!(!with.arguments().iter().any(|a| a == "--settings"));
        }
    }
}

#[test]
fn session_projections_for_all_providers() {
    use crate::providers::SessionProjection;
    use camino::Utf8PathBuf;

    // Claude Code projects only its commands catalog
    assert_eq!(
        providers::session_projections(Provider::ClaudeCode),
        vec![SessionProjection {
            hall_source: Utf8PathBuf::from(".claude/commands"),
            config_relative_dest: Utf8PathBuf::from("commands"),
        }]
    );

    // OpenCode projects only its commands catalog
    assert_eq!(
        providers::session_projections(Provider::OpenCode),
        vec![SessionProjection {
            hall_source: Utf8PathBuf::from(".opencode/commands"),
            config_relative_dest: Utf8PathBuf::from("commands"),
        }]
    );

    // OMP projects commands catalog, hooks/pre, and extensions
    assert_eq!(
        providers::session_projections(Provider::Omp),
        vec![
            SessionProjection {
                hall_source: Utf8PathBuf::from(".omp/commands"),
                config_relative_dest: Utf8PathBuf::from("commands"),
            },
            SessionProjection {
                hall_source: Utf8PathBuf::from(".omp/hooks/pre"),
                config_relative_dest: Utf8PathBuf::from("hooks/pre"),
            },
            SessionProjection {
                hall_source: Utf8PathBuf::from(".omp/extensions"),
                config_relative_dest: Utf8PathBuf::from("extensions"),
            },
        ]
    );
}

use crate::providers::extract_search_pattern;

#[test]
fn extract_search_pattern_reads_grep_and_glob_pattern_field() {
    let input = serde_json::json!({ "pattern": "fn record_miss" });
    assert_eq!(
        extract_search_pattern("Grep", &input),
        Some("fn record_miss".to_owned())
    );
    assert_eq!(
        extract_search_pattern("Glob", &input),
        Some("fn record_miss".to_owned())
    );
}

#[test]
fn extract_search_pattern_reads_rg_and_grep_bash_commands() {
    let rg = serde_json::json!({ "command": "rg 'TODO' src/" });
    assert_eq!(
        extract_search_pattern("Bash", &rg),
        Some("rg 'TODO' src/".to_owned())
    );

    let grep = serde_json::json!({ "command": "grep -rn foo ." });
    assert_eq!(
        extract_search_pattern("Bash", &grep),
        Some("grep -rn foo .".to_owned())
    );

    let rtk_proxy = serde_json::json!({ "command": "rtk proxy rg 'bar' ." });
    assert_eq!(
        extract_search_pattern("Bash", &rtk_proxy),
        Some("rtk proxy rg 'bar' .".to_owned())
    );
}

#[test]
fn extract_search_pattern_finds_rg_after_double_ampersand_and_semicolon() {
    let after_and = serde_json::json!({ "command": "cd src && rg 'TODO'" });
    assert_eq!(
        extract_search_pattern("Bash", &after_and),
        Some("rg 'TODO'".to_owned())
    );

    let after_semicolon = serde_json::json!({ "command": "ls; rtk rg 'baz'" });
    assert_eq!(
        extract_search_pattern("Bash", &after_semicolon),
        Some("rtk rg 'baz'".to_owned())
    );
}

#[test]
fn extract_search_pattern_takes_only_the_first_command_of_a_pipeline() {
    for (command, expected) in [
        ("rg foo | head", Some("rg foo")),
        ("cat x | rg foo", None),
        ("cargo test 2>&1 | grep -E \"Tests \"", None),
        ("a || grep bar", Some("grep bar")),
        ("rtk grep baz", Some("rtk grep baz")),
    ] {
        let input = serde_json::json!({ "command": command });
        assert_eq!(
            extract_search_pattern("Bash", &input).as_deref(),
            expected,
            "{command}"
        );
    }
}

#[test]
fn extract_search_pattern_keeps_quoted_operators_inside_the_pattern() {
    for command in [
        "rg -n 'export (const|function)' src",
        r#"grep -n "foo\|bar" f.rs"#,
        r#"rg -n "a && b; c" src"#,
    ] {
        let input = serde_json::json!({ "command": command });
        assert_eq!(
            extract_search_pattern("Bash", &input).as_deref(),
            Some(command),
            "{command}"
        );
    }
}

#[test]
fn extract_search_pattern_never_reads_heredoc_bodies() {
    let input = serde_json::json!({
        "command": "cat > f.md <<'EOF'\nrg looks like a search\ngrep too\nEOF"
    });
    assert_eq!(extract_search_pattern("Bash", &input), None);
}

#[test]
fn extract_search_pattern_ignores_searches_over_non_code_targets() {
    for command in [
        r#"grep -E "❌" backend.log"#,
        r#"grep -oE '/assets/[^"]+\.js' dist/index.html"#,
        r#"grep -n "^###" /tmp/claude-1000/ph.txt"#,
        "rg -l foo node_modules/react",
        "rg foo target/debug build/out.txt",
    ] {
        let input = serde_json::json!({ "command": command });
        assert_eq!(extract_search_pattern("Bash", &input), None, "{command}");
    }
    for command in ["rg -n foo", "rg -n foo src dist", "grep -rn foo ."] {
        let input = serde_json::json!({ "command": command });
        assert_eq!(
            extract_search_pattern("Bash", &input).as_deref(),
            Some(command),
            "{command}"
        );
    }
}

#[test]
fn extract_search_pattern_ignores_non_search_bash_and_other_tools() {
    let build = serde_json::json!({ "command": "cargo build --release" });
    assert_eq!(extract_search_pattern("Bash", &build), None);
    assert_eq!(extract_search_pattern("Write", &build), None);
}

#[test]
fn extract_search_pattern_truncates_to_500_chars() {
    let long = "a".repeat(600);
    let input = serde_json::json!({ "pattern": long });
    let extracted = extract_search_pattern("Grep", &input).unwrap();
    assert_eq!(extracted.chars().count(), 500);
    assert!(long.starts_with(&extracted));
}

fn bash_search(command: &str) -> Option<String> {
    extract_search_pattern("Bash", &serde_json::json!({ "command": command }))
}

#[test]
fn herestrings_and_shifts_do_not_open_a_heredoc() {
    assert_eq!(
        bash_search("rg x <<< \"$v\"\nrg y").as_deref(),
        Some("rg x <<< \"$v\"")
    );
    assert_eq!(
        bash_search("cat z <<< \"$v\"\nrg y").as_deref(),
        Some("rg y")
    );
    assert_eq!(bash_search("echo $((1<<2))\nrg y").as_deref(), Some("rg y"));
    assert_eq!(bash_search("cat > f <<'EOF'\nrg a\nEOF"), None);
}

#[test]
fn flag_values_are_not_mistaken_for_patterns_or_targets() {
    assert_eq!(bash_search("rg -t log foo build/"), None);
    assert_eq!(bash_search("rg -g '*.rs' foo dist"), None);
    assert_eq!(bash_search("rg -e foo dist"), None);
    assert_eq!(
        bash_search("rg -g '*.rs' foo src").as_deref(),
        Some("rg -g '*.rs' foo src")
    );
}

#[test]
fn ampersand_in_a_redirection_is_not_a_separator() {
    assert_eq!(
        bash_search("rg foo 2>&1 | head").as_deref(),
        Some("rg foo 2>&1")
    );
    assert_eq!(
        bash_search("rg foo &> out.txt").as_deref(),
        Some("rg foo &> out.txt")
    );
}

#[test]
fn comments_are_not_searches() {
    assert_eq!(bash_search("echo hi # rg later"), None);
    assert_eq!(bash_search("# rg old\nls"), None);
    assert_eq!(bash_search("ls # x; rg y"), None);
    assert_eq!(bash_search("rg a#b src").as_deref(), Some("rg a#b src"));
}

#[test]
fn a_non_code_dir_counts_only_as_the_first_path_component() {
    assert_eq!(
        bash_search("rg foo src/build/mod.rs").as_deref(),
        Some("rg foo src/build/mod.rs")
    );
    assert_eq!(bash_search("rg foo build/x"), None);
    assert_eq!(bash_search("rg foo ./build/x"), None);
}
