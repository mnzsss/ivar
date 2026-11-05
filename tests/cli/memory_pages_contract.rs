#![allow(clippy::unwrap_used)]

use ivar::domain::capability::{Support, provider_support};
use ivar::domain::provider::Provider;

use crate::common::{hall_root, ivar};

#[test]
fn every_provider_gets_the_cleanup_command_and_plan_skill_with_the_memory_paths() {
    for provider in Provider::SELECTABLE {
        let support = provider_support(provider);
        assert_eq!(support.cleanup_command, Support::Supported, "{provider}");
        assert_eq!(support.plan_memory_step, Support::Supported, "{provider}");

        let (_guard, root) = hall_root();
        ivar().current_dir(&root).arg("init").assert().success();
        std::fs::write(
            root.join("ivar.json"),
            format!(
                r#"{{"name":"acme","providers":{{"available":["claude-code","{id}"],"default":"{id}"}},"repos":[],"version":1}}"#,
                id = provider.id()
            ),
        )
        .unwrap();
        ivar().current_dir(&root).arg("sync").assert().success();

        let command = std::fs::read_to_string(
            root.join(provider.commands_dir())
                .join("ivar-feature-cleanup.md"),
        )
        .unwrap();
        let skill =
            std::fs::read_to_string(root.join(provider.skills_dir()).join("ivar-plan/SKILL.md"))
                .unwrap();
        assert!(command.contains("docs/memory/update"), "{provider}");
        assert!(skill.contains("ivar memory query"), "{provider}");
        for text in [&command, &skill] {
            assert!(
                !text.contains("docs/updates") && !text.contains("docs/product"),
                "{provider}"
            );
        }
    }
}
