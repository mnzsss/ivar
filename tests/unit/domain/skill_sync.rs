#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;

use camino::Utf8PathBuf;

use super::*;
use crate::domain::name::RepoName;
use crate::domain::skill::{ExternalSkillSource, Skill, SkillFrontmatter, SkillRoot};

fn authored_skill(id: &str, dir: &str) -> Skill {
    Skill::from_frontmatter(
        RepoName::new(id).unwrap(),
        Utf8PathBuf::from(dir),
        SkillFrontmatter::authored(id, format!("The {id} skill")),
        SkillRoot::Hall,
    )
}

fn external_skill(id: &str, dir: &str) -> Skill {
    Skill::from_frontmatter(
        RepoName::new(id).unwrap(),
        Utf8PathBuf::from(dir),
        SkillFrontmatter {
            name: id.to_owned(),
            description: Some(format!("The {id} skill")),
            source: Some(ExternalSkillSource {
                repo: "owner/repo".to_owned(),
                path: format!("skills/{id}"),
                git_ref: "main".to_owned(),
            }),
        },
        SkillRoot::Hall,
    )
}

fn target(
    id: TargetId,
    skill: &Skill,
    path: &str,
    source_hash: &str,
    status: MaterialStatus,
) -> Target {
    Target {
        id,
        skill: skill.id.clone(),
        path: Utf8PathBuf::from(path),
        source_path: skill.dir.clone(),
        source_hash: source_hash.to_owned(),
        status,
    }
}

fn provider(mode: RenderMode) -> ProviderEntry {
    ProviderEntry {
        target_path: Utf8PathBuf::from("/target"),
        rendered_hash: "sha256:x".to_owned(),
        linked_at: "2026-01-01T00:00:00.000Z".to_owned(),
        mode: Some(mode),
    }
}

fn entry(source_hash: &str, provider: ProviderEntry) -> InstallationEntry {
    InstallationEntry {
        source_path: Utf8PathBuf::from("/source"),
        source_hash: source_hash.to_owned(),
        installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
        commit_sha: None,
        providers: {
            let mut map = HashMap::new();
            map.insert(TargetId::Claude, provider);
            map
        },
    }
}

#[test]
fn create_when_target_missing_and_no_state() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::Missing,
    );
    let state = State::default();

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Create);
    assert_eq!(steps[0].mode, RenderMode::Symlink);
}

#[test]
fn adopt_existing_matching_target_when_no_state() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::Ok,
    );
    let state = State::default();

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Create);
    assert_eq!(steps[0].reason.as_deref(), Some("adopt existing target"));
}

#[test]
fn update_when_untracked_obstruction_exists() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::NotLink,
    );
    let state = State::default();

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Update);
    assert_eq!(
        steps[0].reason.as_deref(),
        Some("untracked target entry exists")
    );
}

#[test]
fn update_when_render_mode_changed() {
    let skill = external_skill("ext", "/source/ext");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/ext",
        "sha256:any",
        MaterialStatus::Ok,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "ext".to_owned(),
                entry("sha256:stale", provider(RenderMode::Symlink)),
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Update);
    assert_eq!(steps[0].reason.as_deref(), Some("render mode changed"));
}

#[test]
fn create_when_tracked_target_is_missing() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::Missing,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "alpha".to_owned(),
                entry("sha256:stale", provider(RenderMode::Symlink)),
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Create);
}

#[test]
fn update_when_symlink_points_elsewhere() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::WrongLink,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "alpha".to_owned(),
                entry("sha256:stale", provider(RenderMode::Symlink)),
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Update);
}

#[test]
fn update_when_tracked_target_is_not_a_symlink() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::NotLink,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "alpha".to_owned(),
                entry("sha256:stale", provider(RenderMode::Symlink)),
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Update);
    assert_eq!(steps[0].reason.as_deref(), Some("target is not a symlink"));
}

#[test]
fn unchanged_with_refreshed_hash_when_source_changed() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:new",
        MaterialStatus::Ok,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "alpha".to_owned(),
                entry("sha256:old", provider(RenderMode::Symlink)),
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Unchanged);
    assert_eq!(
        steps[0].reason.as_deref(),
        Some("source changed (hash refreshed)")
    );
}

#[test]
fn unchanged_when_everything_matches() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:same",
        MaterialStatus::Ok,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "alpha".to_owned(),
                entry("sha256:same", provider(RenderMode::Symlink)),
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Unchanged);
    assert!(steps[0].reason.is_none());
}

#[test]
fn remove_skills_no_longer_declared() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::Missing,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "ghost".to_owned(),
                InstallationEntry {
                    source_path: Utf8PathBuf::from("/source/ghost"),
                    source_hash: "sha256:x".to_owned(),
                    installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
                    commit_sha: None,
                    providers: {
                        let mut providers = HashMap::new();
                        providers.insert(
                            TargetId::Claude,
                            ProviderEntry {
                                target_path: Utf8PathBuf::from("/target/ghost"),
                                rendered_hash: "sha256:x".to_owned(),
                                linked_at: "2026-01-01T00:00:00.000Z".to_owned(),
                                mode: Some(RenderMode::Symlink),
                            },
                        );
                        providers
                    },
                },
            );
            map
        },
    };

    let steps = plan(&[skill], &[target], &state);

    // Two steps: create the missing alpha, remove the untracked ghost.
    assert_eq!(steps.len(), 2);
    // Steps are sorted by skill name, so ghost comes after alpha.
    assert_eq!(steps[0].action, Action::Create);
    assert_eq!(steps[0].skill.as_str(), "alpha");
    assert_eq!(steps[1].skill.as_str(), "ghost");
    assert_eq!(steps[1].action, Action::Remove);
    assert_eq!(steps[1].target.as_str(), "/target/ghost");
}

#[test]
fn commit_sha_short_circuit_skips() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Claude,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::Missing,
    );
    let state = State {
        installations: {
            let mut map = HashMap::new();
            map.insert(
                "alpha".to_owned(),
                InstallationEntry {
                    source_path: Utf8PathBuf::from("/source/alpha"),
                    source_hash: "sha256:stale".to_owned(),
                    installed_at: "2026-01-01T00:00:00.000Z".to_owned(),
                    commit_sha: Some("abc123".to_owned()),
                    providers: {
                        let mut providers = HashMap::new();
                        providers.insert(TargetId::Claude, provider(RenderMode::Symlink));
                        providers
                    },
                },
            );
            map
        },
    };

    let steps = plan_with_options(
        &[skill],
        &[target],
        &state,
        PlanOptions {
            repo_head: Some("abc123".to_owned()),
            tree_clean: true,
        },
    );

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Unchanged);
    assert_eq!(steps[0].reason.as_deref(), Some("commit-sha unchanged"));
}
#[test]
fn omp_target_lifecycle_round_trip() {
    let skill = authored_skill("alpha", "/source/alpha");
    let target = target(
        TargetId::Omp,
        &skill,
        "/target/alpha",
        "sha256:any",
        MaterialStatus::Missing,
    );
    let state = State::default();

    let steps = plan(&[skill], &[target], &state);

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, Action::Create);
    assert_eq!(steps[0].mode, RenderMode::Symlink);
    assert_eq!(TargetId::Omp.as_str(), "omp");
}
