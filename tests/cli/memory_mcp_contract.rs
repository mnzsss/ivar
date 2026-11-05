#![allow(clippy::unwrap_used)]

use ivar::domain::capability::{Support, provider_support};
use ivar::domain::provider::Provider;

use crate::common::{hall_root, ivar};

fn sync_hall(provider: Provider) -> (tempfile::TempDir, std::path::PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let mut available = vec!["claude-code", provider.id()];
    available.dedup();
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest.insert("name".to_owned(), "acme".into());
    manifest.insert(
        "providers".to_owned(),
        serde_json::json!({"available": available, "default": provider.id()}),
    );
    manifest.insert(
        "mcp".to_owned(),
        serde_json::json!([{"name": "graph", "type": "local", "command": "ivar", "args": ["graph", "mcp"]}]),
    );
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    ivar().current_dir(&root).arg("sync").assert().success();
    (guard, root.into())
}

#[test]
fn every_provider_gets_the_ivar_server_and_the_planning_memory_steps() {
    for provider in Provider::SELECTABLE {
        let support = provider_support(provider);
        assert_eq!(support.memory_mcp, Support::Supported, "{provider}");
        assert_eq!(support.planning_memory, Support::Supported, "{provider}");
        let (_guard, root) = sync_hall(provider);

        let manifest = std::fs::read_to_string(root.join("ivar.json")).unwrap();
        assert!(
            manifest.contains(r#""name": "ivar""#) || manifest.contains(r#""name":"ivar""#),
            "{provider}"
        );
        assert!(!manifest.contains(r#""name": "graph""#), "{provider}");

        let config = std::fs::read_to_string(root.join(provider.mcp_config_path())).unwrap();
        assert!(config.contains("acme-ivar"), "{provider}");
        assert!(!config.contains("acme-graph"), "{provider}");

        for (skill, needles) in [
            (
                "ivar-explore",
                &["Conhecimento prévio", "ivar memory query"][..],
            ),
            (
                "ivar-plan",
                &["one `decision` per trade-off", "memory_write_page"][..],
            ),
            (
                "ivar-execute",
                &["**Memory refs:**", "only the coordinator writes"][..],
            ),
        ] {
            let text = std::fs::read_to_string(
                root.join(provider.skills_dir())
                    .join(skill)
                    .join("SKILL.md"),
            )
            .unwrap();
            for needle in needles {
                assert!(text.contains(needle), "{provider} {skill} {needle}");
            }
        }
        let subagent = std::fs::read_to_string(
            root.join(provider.skills_dir())
                .join("ivar-execute/references/subagent.md"),
        )
        .unwrap();
        assert!(
            subagent.contains("Learnings") && subagent.contains("Memory refs"),
            "{provider}"
        );
    }
}

#[test]
fn claude_code_approves_the_ivar_server_and_not_the_graph_one() {
    let (_guard, root) = sync_hall(Provider::ClaudeCode);
    let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
    assert!(settings.contains("enabledMcpjsonServers"));
    assert!(settings.contains("acme-ivar"));
    assert!(!settings.contains("acme-graph"));
}

#[test]
fn memory_lint_exits_non_zero_on_an_error_and_zero_on_a_clean_hall() {
    let (_guard, root) = sync_hall(Provider::ClaudeCode);
    ivar()
        .current_dir(&root)
        .args(["memory", "lint"])
        .assert()
        .success();
    std::fs::create_dir_all(root.join("docs/memory/decision")).unwrap();
    std::fs::write(
        root.join("docs/memory/decision/a.md"),
        "---\nconfidence: high\n---\nx\n",
    )
    .unwrap();
    ivar()
        .current_dir(&root)
        .args(["memory", "lint", "--json"])
        .assert()
        .failure()
        .stdout(predicates::str::contains("lint.field_unknown"));
}
