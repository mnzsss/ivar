//! Golden-vector tests for the skill sync planner.
//!
//! Each JSON fixture under `tests/golden/skill_sync/` contains an input
//! (skills, targets, state) and the expected steps output. The test harness
//! loads every fixture, runs the pure sync-plan reducer against it, and
//! asserts byte-for-byte match on the canonicalised JSON output.
//!
//! One fixture (`tampered.json`) carries intentionally wrong expected output —
//! it exists to verify that the test framework actually fails when the code
//! produces a different result. See `scripts/differential.sh`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use ivar::domain::name::RepoName;
use ivar::domain::skill::{ExternalRef, RenderMode, Skill, SkillRoot, Source};
use ivar::domain::skill_sync::{
    Action, InstallationEntry, MaterialStatus, PlanOptions, ProviderEntry, State, Step, Target,
    TargetId,
};

// -- fixture types ----------------------------------------------------------

/// A single golden-vector fixture on disk.
#[derive(Debug, Deserialize)]
struct Fixture {
    input: FixtureInput,
    /// The steps the reducer must produce for this fixture.
    expected_steps: Vec<GoldenStep>,
}

#[derive(Debug, Deserialize)]
struct FixtureInput {
    skills: Vec<GoldenSkill>,
    targets: Vec<GoldenTarget>,
    state: GoldenState,
    options: Option<GoldenOptions>,
}

#[derive(Debug, Deserialize)]
struct GoldenSkill {
    id: String,
    description: String,
    source: String, // "authored" | "external"
    #[serde(default)]
    external_source: Option<GoldenExternalSource>,
    dir: Utf8PathBuf,
}

#[derive(Debug, Deserialize)]
struct GoldenExternalSource {
    repo: String,
    path: String,
    git_ref: String,
}

#[derive(Debug, Deserialize)]
struct GoldenTarget {
    id: String,
    skill: String,
    path: Utf8PathBuf,
    source_path: Utf8PathBuf,
    source_hash: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct GoldenState {
    installations: HashMap<String, GoldenInstallation>,
}

#[derive(Debug, Deserialize)]
struct GoldenInstallation {
    source_path: Utf8PathBuf,
    source_hash: String,
    installed_at: String,
    commit_sha: Option<String>,
    providers: HashMap<String, GoldenProvider>,
}

#[derive(Debug, Deserialize)]
struct GoldenProvider {
    target_path: Utf8PathBuf,
    rendered_hash: String,
    linked_at: String,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoldenOptions {
    repo_head: Option<String>,
    tree_clean: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct GoldenStep {
    action: String,
    mode: String,
    reason: Option<String>,
    skill: String,
    source: Utf8PathBuf,
    target: Utf8PathBuf,
}

// -- conversion helpers -----------------------------------------------------

fn parse_mode(s: &str) -> RenderMode {
    match s {
        "symlink" => RenderMode::Symlink,
        "copy" => RenderMode::Copy,
        other => panic!("unknown mode '{other}'"),
    }
}

fn parse_target_id(s: &str) -> TargetId {
    match s {
        "claude" => TargetId::Claude,
        "opencode" => TargetId::OpenCode,
        other => panic!("unknown target id '{other}'"),
    }
}

fn parse_material_status(s: &str) -> MaterialStatus {
    match s {
        "missing" => MaterialStatus::Missing,
        "ok" => MaterialStatus::Ok,
        "wrong_link" => MaterialStatus::WrongLink,
        "not_link" => MaterialStatus::NotLink,
        "broken_symlink" => MaterialStatus::BrokenSymlink,
        other => panic!("unknown material status '{other}'"),
    }
}

fn build_skill(g: &GoldenSkill) -> Skill {
    let source = match g.source.as_str() {
        "authored" => Source::Authored,
        "external" => Source::External(ExternalRef {
            repo: g.external_source.as_ref().unwrap().repo.clone(),
            path: g.external_source.as_ref().unwrap().path.clone(),
            git_ref: g.external_source.as_ref().unwrap().git_ref.clone(),
        }),
        other => panic!("unknown skill source type '{other}'"),
    };
    Skill {
        id: RepoName::new(&g.id).expect("valid repo name"),
        description: g.description.clone(),
        source,
        root: SkillRoot::Hall,
        dir: g.dir.clone(),
    }
}

fn build_target(g: &GoldenTarget) -> Target {
    Target {
        id: parse_target_id(&g.id),
        skill: RepoName::new(&g.skill).expect("valid repo name"),
        path: g.path.clone(),
        source_path: g.source_path.clone(),
        source_hash: g.source_hash.clone(),
        status: parse_material_status(&g.status),
    }
}

fn build_state(g: &GoldenState) -> State {
    let installations = g
        .installations
        .iter()
        .map(|(key, val)| {
            let providers: HashMap<TargetId, ProviderEntry> = val
                .providers
                .iter()
                .map(|(k, v)| {
                    (
                        parse_target_id(k),
                        ProviderEntry {
                            target_path: v.target_path.clone(),
                            rendered_hash: v.rendered_hash.clone(),
                            linked_at: v.linked_at.clone(),
                            mode: v.mode.as_deref().map(parse_mode),
                        },
                    )
                })
                .collect();
            (
                key.clone(),
                InstallationEntry {
                    source_path: val.source_path.clone(),
                    source_hash: val.source_hash.clone(),
                    installed_at: val.installed_at.clone(),
                    commit_sha: val.commit_sha.clone(),
                    providers,
                },
            )
        })
        .collect();
    State { installations }
}

fn build_options(g: &GoldenOptions) -> PlanOptions {
    PlanOptions {
        repo_head: g.repo_head.clone(),
        tree_clean: g.tree_clean,
    }
}

fn step_to_golden(step: &Step) -> GoldenStep {
    let action = match step.action {
        Action::Create => "create",
        Action::Update => "update",
        Action::Remove => "remove",
        Action::Unchanged => "unchanged",
    };
    let mode = match step.mode {
        RenderMode::Symlink => "symlink",
        RenderMode::Copy => "copy",
    };
    GoldenStep {
        action: action.to_owned(),
        mode: mode.to_owned(),
        reason: step.reason.clone(),
        skill: step.skill.as_str().to_owned(),
        source: step.source.clone(),
        target: step.target.clone(),
    }
}

// -- test harness -----------------------------------------------------------

fn load_fixture(name: &str) -> Fixture {
    let path = format!(
        "{}/tests/golden/skill_sync/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse fixture {name}: {e}"))
}

fn run_plan(fixture: &Fixture) -> Result<Vec<Step>, String> {
    let skills: Vec<Skill> = fixture.input.skills.iter().map(build_skill).collect();
    let targets: Vec<Target> = fixture.input.targets.iter().map(build_target).collect();
    let state = build_state(&fixture.input.state);

    let steps = match &fixture.input.options {
        Some(opts) => ivar::domain::skill_sync::plan_with_options(
            &skills,
            &targets,
            &state,
            build_options(opts),
        ),
        None => ivar::domain::skill_sync::plan(&skills, &targets, &state),
    };

    Ok(steps)
}

fn steps_to_canonical(steps: &[Step]) -> String {
    let golden: Vec<GoldenStep> = steps.iter().map(step_to_golden).collect();
    ivar::infra::json::to_canonical_string(&golden).unwrap()
}

// -- individual tests per fixture --------------------------------------------

#[test]
fn create_vector_matches_golden() {
    let fixture = load_fixture("create");
    let steps = run_plan(&fixture).expect("plan should succeed");
    let actual = steps_to_canonical(&steps);
    let expected = ivar::infra::json::to_canonical_string(&fixture.expected_steps).unwrap();
    assert_eq!(actual, expected, "create vector mismatch");
}

#[test]
fn update_not_link_vector_matches_golden() {
    let fixture = load_fixture("update_not_link");
    let steps = run_plan(&fixture).expect("plan should succeed");
    let actual = steps_to_canonical(&steps);
    let expected = ivar::infra::json::to_canonical_string(&fixture.expected_steps).unwrap();
    assert_eq!(actual, expected, "update_not_link vector mismatch");
}

#[test]
fn remove_vector_matches_golden() {
    let fixture = load_fixture("remove");
    let steps = run_plan(&fixture).expect("plan should succeed");
    let actual = steps_to_canonical(&steps);
    let expected = ivar::infra::json::to_canonical_string(&fixture.expected_steps).unwrap();
    assert_eq!(actual, expected, "remove vector mismatch");
}

#[test]
fn unchanged_vector_matches_golden() {
    let fixture = load_fixture("unchanged");
    let steps = run_plan(&fixture).expect("plan should succeed");
    let actual = steps_to_canonical(&steps);
    let expected = ivar::infra::json::to_canonical_string(&fixture.expected_steps).unwrap();
    assert_eq!(actual, expected, "unchanged vector mismatch");
}

#[test]
fn tampered_vector_fails_as_expected() {
    // This test verifies that the test framework catches mismatches.
    // The tampered.json fixture has deliberately wrong expected output.
    // If this test passes, the golden vector was accidentally fixed —
    // which means the self-test mechanism is broken.
    let fixture = load_fixture("tampered");
    let steps = run_plan(&fixture).expect("plan should succeed");
    let actual = steps_to_canonical(&steps);
    let expected = ivar::infra::json::to_canonical_string(&fixture.expected_steps).unwrap();
    assert_ne!(
        actual, expected,
        "tampered vector must NOT match — if it does, the golden vector was fixed and the self-test is broken"
    );
}

#[test]
fn all_fixtures_are_accounted_for() {
    // Ensure no golden vector files are left untested.
    let golden_dir = format!("{}/tests/golden/skill_sync", env!("CARGO_MANIFEST_DIR"));
    let entries = std::fs::read_dir(&golden_dir)
        .unwrap_or_else(|e| panic!("cannot read golden directory {golden_dir}: {e}"));

    let mut tested = std::collections::HashSet::new();
    tested.insert("create".to_owned());
    tested.insert("update_not_link".to_owned());
    tested.insert("remove".to_owned());
    tested.insert("unchanged".to_owned());
    tested.insert("tampered".to_owned());

    for entry in entries {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".json") {
            let base = name.strip_suffix(".json").unwrap();
            assert!(tested.contains(base), "untested golden fixture: {base}");
        }
    }
}
