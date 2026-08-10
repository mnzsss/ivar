#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;

use camino::Utf8PathBuf;

use super::*;
use crate::domain::skill::{ExternalRef, RenderMode};
use crate::domain::skill_sync::{InstallationEntry, ProviderEntry};
use crate::test_support::utf8_temp_dir;

// -- parse_frontmatter ----------------------------------------------------

#[test]
fn parses_authored_skill_frontmatter() {
    let content = "---\nname: my-skill\ndescription: Does cool things\n---\nbody\n";
    let result = parse_frontmatter(content).unwrap().unwrap();
    assert_eq!(result.name, "my-skill");
    assert_eq!(result.description, Some("Does cool things".to_owned()));
    assert!(result.source.is_none());
}

#[test]
fn parses_external_skill_frontmatter() {
    let content = "---\nname: ext-skill\nsource:\n  repo: owner/repo\n  path: skills/ext\n  ref: v1.0\n---\nbody\n";
    let result = parse_frontmatter(content).unwrap().unwrap();
    assert_eq!(result.name, "ext-skill");
    assert_eq!(
        result.source,
        Some(ExternalRef {
            repo: "owner/repo".to_owned(),
            path: "skills/ext".to_owned(),
            git_ref: "v1.0".to_owned(),
        })
    );
}

#[test]
fn no_frontmatter_returns_none() {
    let content = "just body text\n";
    let result = parse_frontmatter(content).unwrap();
    assert!(result.is_none());
}

#[test]
fn empty_frontmatter_returns_none() {
    let content = "---\n---\nbody\n";
    let result = parse_frontmatter(content).unwrap();
    assert!(result.is_none());
}

#[test]
fn frontmatter_without_name_returns_blocked_error() {
    let content = "---\ndescription: no name here\n---\nbody\n";
    let error = parse_frontmatter(content).unwrap_err();
    assert_eq!(error.code, "skill.missing_name");
    assert_eq!(error.status, crate::error::Status::Blocked);
}

#[test]
fn malformed_yaml_returns_failed_error() {
    // An unclosed flow mapping is invalid YAML.
    let content = "---\nname: [broken\n---\nbody\n";
    let error = parse_frontmatter(content).unwrap_err();
    assert_eq!(error.code, "skill.parse_error");
    assert_eq!(error.status, crate::error::Status::Failed);
}

// -- round-trip: write, read back, exact bytes ---------------------------

#[test]
fn empty_state_round_trips() {
    let (_dir, root) = utf8_temp_dir();
    let state = State::default();

    write(&root, &state).unwrap();

    let read_back = read(&root).unwrap().unwrap();
    assert_eq!(read_back.installations.len(), 0);
}

#[test]
fn state_with_installations_round_trips() {
    let (_dir, root) = utf8_temp_dir();
    let mut installations = HashMap::new();
    installations.insert(
        "alpha".to_owned(),
        InstallationEntry {
            source_path: Utf8PathBuf::from("/hall/.ivar/skills/alpha"),
            source_hash: "sha256:abc123".to_owned(),
            installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
            commit_sha: Some("deadbeef".to_owned()),
            providers: {
                let mut map = HashMap::new();
                map.insert(
                    TargetId::Claude,
                    ProviderEntry {
                        target_path: Utf8PathBuf::from("/hall/.claude/skills/alpha"),
                        rendered_hash: "sha256:x".to_owned(),
                        linked_at: "2026-01-01T00:00:00.000Z".to_owned(),
                        mode: Some(RenderMode::Symlink),
                    },
                );
                map.insert(
                    TargetId::OpenCode,
                    ProviderEntry {
                        target_path: Utf8PathBuf::from("/hall/.opencode/skills/alpha"),
                        rendered_hash: "sha256:y".to_owned(),
                        linked_at: "2026-01-01T00:00:00.000Z".to_owned(),
                        mode: Some(RenderMode::Copy),
                    },
                );
                map
            },
        },
    );
    let state = State { installations };

    write(&root, &state).unwrap();

    let read_back = read(&root).unwrap().unwrap();
    assert_eq!(read_back, state);
}

#[test]
fn absent_state_file_returns_none() {
    let (_dir, root) = utf8_temp_dir();
    let result = read(&root).unwrap();
    assert!(result.is_none());
}

#[test]
fn broken_json_is_a_hard_error() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join(".ivar").join("skills").join("state.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();

    let error = read(&root).unwrap_err();
    assert!(matches!(error, Error::Json(json::Error::Parse { .. })));
}

// -- target_path helper ---------------------------------------------------

#[test]
fn target_path_claude() {
    let path = target_path(TargetId::Claude, "my-skill");
    assert_eq!(path.as_str(), ".claude/skills/my-skill");
}

#[test]
fn target_path_opencode() {
    let path = target_path(TargetId::OpenCode, "my-skill");
    assert_eq!(path.as_str(), ".opencode/skills/my-skill");
}
