#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

// -- Skill construction ---------------------------------------------------

#[test]
fn authored_skill_has_no_source() {
    let skill = Skill::from_frontmatter(
        RepoName::new("alpha").unwrap(),
        camino::Utf8PathBuf::from("/skills/alpha"),
        SkillFrontmatter {
            name: "alpha".to_owned(),
            description: None,
            source: None,
        },
        SkillRoot::Hall,
    );
    assert_eq!(skill.render_mode(), RenderMode::Symlink);
}

#[test]
fn external_skill_has_an_external_source() {
    let skill = Skill::from_frontmatter(
        RepoName::new("beta").unwrap(),
        camino::Utf8PathBuf::from("/skills/beta"),
        SkillFrontmatter {
            name: "beta".to_owned(),
            description: Some("An external skill".to_owned()),
            source: Some(ExternalRef {
                repo: "org/toolkit".to_owned(),
                path: "skills/beta".to_owned(),
                git_ref: "main".to_owned(),
            }),
        },
        SkillRoot::Hall,
    );
    assert_eq!(skill.render_mode(), RenderMode::Copy);
}

#[test]
fn default_description_is_generated_from_id() {
    let skill = Skill::from_frontmatter(
        RepoName::new("gamma").unwrap(),
        camino::Utf8PathBuf::from("/skills/gamma"),
        SkillFrontmatter {
            name: "gamma".to_owned(),
            description: None,
            source: None,
        },
        SkillRoot::Hall,
    );
    assert_eq!(skill.description, "The gamma skill");
}

// -- Round-trip serialization ---------------------------------------------

#[test]
fn serialize_and_deserialize_authored_skill() {
    let original = Skill {
        id: RepoName::new("roundtrip").unwrap(),
        description: "A round-trip skill".to_owned(),
        source: Source::Authored,
        root: SkillRoot::Hall,
        dir: camino::Utf8PathBuf::from("/skills/roundtrip"),
    };

    let json = serde_json::to_string(&original).unwrap();
    let restored: Skill = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, original);
}

#[test]
fn serialize_and_deserialize_external_skill() {
    let original = Skill {
        id: RepoName::new("ext-rt").unwrap(),
        description: "External skill".to_owned(),
        source: Source::External(ExternalRef {
            repo: "owner/repo".to_owned(),
            path: "skills/ext".to_owned(),
            git_ref: "abc123".to_owned(),
        }),
        root: SkillRoot::Hall,
        dir: camino::Utf8PathBuf::from("/skills/ext-rt"),
    };

    let json = serde_json::to_string(&original).unwrap();
    let restored: Skill = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, original);
}

#[test]
fn render_mode_matches_source_type() {
    let authored = Skill {
        id: RepoName::new("a").unwrap(),
        description: "a".to_owned(),
        source: Source::Authored,
        root: SkillRoot::Hall,
        dir: camino::Utf8PathBuf::from("/a"),
    };
    assert_eq!(authored.render_mode(), RenderMode::Symlink);

    let external = Skill {
        id: RepoName::new("b").unwrap(),
        description: "b".to_owned(),
        source: Source::External(ExternalRef {
            repo: "x/y".to_owned(),
            path: "p".to_owned(),
            git_ref: "z".to_owned(),
        }),
        root: SkillRoot::Hall,
        dir: camino::Utf8PathBuf::from("/b"),
    };
    assert_eq!(external.render_mode(), RenderMode::Copy);
}
