#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::session::guard::WritableSet;
use crate::action::session::sandbox::{Sandbox, sandbox_temp_root};
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::SessionState;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seed_protected_hall_paths, seeded_repo};
use camino::{Utf8Path, Utf8PathBuf};

fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = crate::action::Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        crate::domain::name::HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    (guard, root)
}

#[test]
fn sandbox_roots_contain_writable_set_bare_git_dev_null_temp_and_provider_dirs() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();
    let sandbox = Sandbox::from_writable_set(
        &set,
        &layout,
        Some(&feature),
        Provider::ClaudeCode,
        &session_id,
    )
    .unwrap();
    let roots = sandbox.roots();

    // 1. Every WritableSet root is present.
    for r in set.roots().unwrap() {
        assert!(roots.contains(&r), "missing WritableSet root {r}");
    }

    // 2. Promoted repo bare git directory is present.
    let bare = layout.repo_bare(&RepoName::new("api").unwrap());
    let bare_canonical = bare.canonicalize_utf8().unwrap_or(bare);
    assert!(
        roots.iter().any(|p| p == &bare_canonical),
        "missing bare git root {bare_canonical}"
    );

    // 3. /dev/null is present if it exists.
    let dev_null = Utf8PathBuf::from("/dev/null");
    if dev_null.exists() {
        let canonical_dev_null = dev_null.canonicalize_utf8().unwrap_or(dev_null);
        assert!(
            roots.iter().any(|p| p == &canonical_dev_null),
            "missing /dev/null"
        );
    }

    // 4. The private temp root is present, the temp dir holding the hall is not.
    let temp = system_temp_dir();
    let private = temp.join(format!("ivar-{session_id}"));
    assert!(
        roots.contains(&private),
        "missing private temp root {private}"
    );
    assert!(
        !roots.contains(&temp),
        "the temp dir holding the hall is a root"
    );
}

#[test]
fn sandbox_discovery_session_derives_roots_without_feature() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let sandbox =
        Sandbox::from_writable_set(&set, &layout, None, Provider::Omp, &session_id).unwrap();
    let roots = sandbox.roots();

    for r in set.roots().unwrap() {
        assert!(roots.contains(&r), "missing discovery WritableSet root {r}");
    }
}

#[test]
fn discovery_sandbox_contains_canonical_hall_sources() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000004").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::infra::fs::write_text(&layout.root().join("HALL.md"), "# Hall\n").unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills()).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills_local()).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let sandbox =
        Sandbox::from_writable_set(&set, &layout, None, Provider::ClaudeCode, &session_id).unwrap();

    assert!(
        sandbox
            .roots()
            .contains(&layout.root().join("HALL.md").canonicalize_utf8().unwrap())
    );
    assert!(
        sandbox
            .roots()
            .contains(&layout.hall_skills().canonicalize_utf8().unwrap())
    );
    assert!(
        sandbox
            .roots()
            .contains(&layout.hall_skills_local().canonicalize_utf8().unwrap())
    );
    assert!(
        sandbox
            .roots()
            .contains(&layout.hall_setups().canonicalize_utf8().unwrap())
    );
    assert!(
        !sandbox
            .roots()
            .contains(&layout.root().canonicalize_utf8().unwrap())
    );
}

#[test]
fn sandbox_grants_hall_root_entries_but_never_covers_the_repos_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000015").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::infra::fs::ensure_dir(&layout.root().join("docs")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let sandbox =
        Sandbox::from_writable_set(&set, &layout, None, Provider::ClaudeCode, &session_id).unwrap();
    let repos = layout.repos_dir().canonicalize_utf8().unwrap();

    assert!(
        sandbox
            .roots()
            .contains(&layout.root().join("docs").canonicalize_utf8().unwrap())
    );
    assert!(
        !sandbox.roots().iter().any(|r| repos.starts_with(r)),
        "no sandbox root may cover .ivar/repos: {:?}",
        sandbox.roots()
    );
}

#[cfg(unix)]
#[test]
fn symlinked_hall_root_entries_stay_out_of_the_kernel_roots() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view_dir =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000017").unwrap());
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside = Utf8PathBuf::try_from(outside.path().to_path_buf())
        .unwrap()
        .canonicalize_utf8()
        .unwrap();
    let default_worktree = layout
        .repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("main").unwrap(),
        )
        .canonicalize_utf8()
        .unwrap();
    symlink(&outside, root.join("outside")).unwrap();
    symlink(&default_worktree, root.join("api-link")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let roots = set.roots().unwrap();
    let canonical_root = root.canonicalize_utf8().unwrap();

    for excluded in [
        outside,
        default_worktree,
        canonical_root.join("outside"),
        canonical_root.join("api-link"),
    ] {
        assert!(!roots.contains(&excluded), "{excluded} in {roots:?}");
    }
}

#[test]
fn sandbox_roots_never_cover_git_hooks_git_config_or_provider_hook_config() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000016").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    seed_protected_hall_paths(&root);

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let sandbox =
        Sandbox::from_writable_set(&set, &layout, None, Provider::ClaudeCode, &session_id).unwrap();
    let canonical_root = root.canonicalize_utf8().unwrap();

    for protected in [
        ".git/hooks",
        ".git/config",
        ".claude/settings.json",
        ".opencode/plugins",
        ".omp/hooks",
    ]
    .map(|path| canonical_root.join(path))
    {
        assert!(
            !sandbox
                .roots()
                .iter()
                .any(|r| protected.starts_with(r) || r.starts_with(&protected)),
            "a sandbox root covers {protected}: {:?}",
            sandbox.roots()
        );
    }
    assert!(
        sandbox
            .roots()
            .contains(&canonical_root.join(".git/objects"))
    );
}

#[test]
fn sandbox_status_enum_variants_and_predicates() {
    use crate::action::session::sandbox::SandboxStatus;

    let enforced = SandboxStatus::Enforced;
    assert!(enforced.is_enforced());

    let degraded = SandboxStatus::Degraded {
        reason: "Partially enforced ABI".into(),
    };
    assert!(!degraded.is_enforced());

    let unavailable = SandboxStatus::Unavailable {
        reason: "Landlock not supported on this platform".into(),
    };
    assert!(!unavailable.is_enforced());
}

#[test]
fn sandbox_status_enum_variants_and_display() {
    use crate::action::session::sandbox::SandboxStatus;

    let enforced = SandboxStatus::Enforced;
    assert!(enforced.is_enforced());

    let degraded = SandboxStatus::Degraded {
        reason: "Partially enforced".into(),
    };
    assert!(!degraded.is_enforced());

    let unavailable = SandboxStatus::Unavailable {
        reason: "Landlock not supported on this platform".into(),
    };
    assert!(!unavailable.is_enforced());
}

#[test]
fn launcher_resolves_session_from_disk_and_builds_sandbox() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let state = SessionState::new(Provider::ClaudeCode, "2026-08-07T12:34:56.000000000Z");
    state.write(&view_dir).unwrap();

    let session_ref =
        crate::action::session::lookup::resolve(&layout, Some(session_id.as_str()), None).unwrap();
    assert_eq!(session_ref.id, session_id);
}

#[test]
fn launcher_empty_command_fails_gracefully() {
    let ctx = crate::action::Ctx::new(Utf8PathBuf::from("/tmp"));
    let result = crate::action::session::sandbox::run_launcher(&ctx, "some-session", false, &[]);
    let err = result.expect_err("empty command should return error");
    assert_eq!(err.code, "sandbox.launcher_missing_command");
}

#[test]
fn launcher_refuses_a_program_that_is_not_the_session_provider() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-08-07T12:34:56.000000000Z")
        .write(&view_dir)
        .unwrap();

    let ctx = crate::action::Ctx::new(root);
    let error = crate::action::session::sandbox::run_launcher(
        &ctx,
        session_id.as_str(),
        false,
        &["opencode".to_owned()],
    )
    .expect_err("the launcher runs this session's provider, nothing else");

    assert_eq!(error.code, "sandbox.launcher_foreign_program");
}

#[test]
#[cfg(target_os = "linux")]
fn sandbox_apply_is_enforced_when_the_kernel_supports_the_requested_abi() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();
    let sandbox = Sandbox::from_writable_set(
        &set,
        &layout,
        Some(&feature),
        Provider::ClaudeCode,
        &session_id,
    )
    .unwrap();

    // Landlock restricts only the calling thread: this test's own.
    let status = sandbox.apply().unwrap();
    if kernel_supports_landlock_scopes() {
        assert_eq!(
            status,
            crate::action::session::sandbox::SandboxStatus::Enforced
        );
    }
}

#[cfg(target_os = "linux")]
fn kernel_supports_landlock_scopes() -> bool {
    use landlock::{ABI, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, Scope};
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_write(ABI::V5))
        .and_then(|r| r.scope(Scope::Signal | Scope::AbstractUnixSocket))
        .and_then(|r| r.create())
        .is_ok()
}

// Note on `sandbox.process_failed` testability at this seam:
// Finding 2 requested test coverage for the non-zero exit branch returning `sandbox.process_failed`.
// At this unit test seam (`run_launcher`), the command is built via `launch::provider_command`
// from the session provider recorded in `state.json`. `ensure_provider_binary` strictly guards
// that `argv[0]` matches the session provider (`claude` / `opencode` / `omp`), while `provider_command`
// uses the provider's fixed binary name. Therefore, reaching `crate::infra::proc::exec` would attempt
// to spawn the real provider executable on the host system (e.g. `claude`).
// In unit tests:
// 1. We cannot rely on `claude` or `opencode` binaries existing on arbitrary test runners.
// 2. We must never spawn or execute a real provider during unit testing.
// 3. Even if a fake binary were placed on PATH, on Linux `exec` replaces the test process via `execve`.
// 4. On non-Linux targets (`#[cfg(not(target_os = "linux"))]`), `exec` falls back to `status().code()`,
//    but still requires spawning a real executable that exits non-zero without being able to mock
//    `provider_command`'s binary name without violating `ensure_provider_binary`.
// Thus, `sandbox.process_failed` cannot be driven from `run_launcher` in unit tests without executing
// a real provider binary or introducing test-only seams into production code.

fn system_temp_dir() -> Utf8PathBuf {
    Utf8PathBuf::try_from(std::env::temp_dir())
        .unwrap()
        .canonicalize_utf8()
        .unwrap()
}

#[test]
fn a_temp_dir_holding_the_hall_narrows_to_a_private_session_dir() {
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000021").unwrap();
    let root = sandbox_temp_root(
        Utf8Path::new("/tmp"),
        &[Utf8PathBuf::from("/tmp/work/hall")],
        &id,
    );
    assert_eq!(
        root,
        Utf8PathBuf::from("/tmp/ivar-6f0c9d5f-0000-4000-8000-000000000021")
    );
}

#[test]
fn a_temp_dir_inside_the_hall_narrows_to_a_private_session_dir() {
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000024").unwrap();
    let root = sandbox_temp_root(
        Utf8Path::new("/home/u/hall/.git"),
        &[Utf8PathBuf::from("/home/u/hall")],
        &id,
    );
    assert_eq!(
        root,
        Utf8PathBuf::from("/home/u/hall/.git/ivar-6f0c9d5f-0000-4000-8000-000000000024")
    );
}

#[test]
fn a_temp_dir_outside_the_hall_is_kept_whole() {
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000022").unwrap();
    let root = sandbox_temp_root(
        Utf8Path::new("/var/tmp"),
        &[
            Utf8PathBuf::from("/home/u/hall"),
            Utf8PathBuf::from("/home/u/hall/.env"),
        ],
        &id,
    );
    assert_eq!(root, Utf8PathBuf::from("/var/tmp"));
}

#[test]
fn a_temp_dir_holding_a_symlinked_protected_path_narrows_to_a_private_session_dir() {
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000027").unwrap();
    let root = sandbox_temp_root(
        Utf8Path::new("/tmp"),
        &[
            Utf8PathBuf::from("/home/u/hall"),
            Utf8PathBuf::from("/tmp/dotfiles/hall.env"),
        ],
        &id,
    );
    assert_eq!(
        root,
        Utf8PathBuf::from("/tmp/ivar-6f0c9d5f-0000-4000-8000-000000000027")
    );
}

// The test hall lives under the system temp dir (tempfile), so the narrowing always applies.
#[test]
fn the_temp_dir_holding_the_hall_is_never_a_sandbox_root() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000025").unwrap();
    let view_dir = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();

    let sandbox =
        Sandbox::from_writable_set(&set, &layout, None, Provider::ClaudeCode, &id).unwrap();
    let private = system_temp_dir().join(format!("ivar-{id}"));

    assert_eq!(sandbox.temp_root(), private);
    assert!(sandbox.roots().contains(&private));
    assert!(
        !sandbox.roots().iter().any(|r| root.starts_with(r)),
        "{:?}",
        sandbox.roots()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&private).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    std::fs::remove_dir_all(&private).unwrap();
}

#[cfg(unix)]
#[test]
fn a_planted_symlink_at_the_private_temp_root_fails_closed() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000026").unwrap();
    let view_dir = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let private = system_temp_dir().join(format!("ivar-{id}"));
    let _ = std::fs::remove_file(&private).or_else(|_| std::fs::remove_dir_all(&private));
    std::os::unix::fs::symlink(&root, &private).unwrap();

    let error = Sandbox::from_writable_set(&set, &layout, None, Provider::ClaudeCode, &id)
        .expect_err("a symlink must never redirect the temp grant");
    std::fs::remove_file(&private).unwrap();

    assert_eq!(error.code, "sandbox.untrusted_temp_root");
}

#[cfg(unix)]
#[test]
fn a_dangling_symlinked_protected_path_is_judged_where_it_resolves() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let (_outside_guard, outside) = crate::test_support::canonical_temp_dir();
    std::os::unix::fs::symlink(outside.join("hall.env"), root.join(".env")).unwrap();

    let paths = super::hall_paths(&layout);

    assert!(paths.contains(&outside.join("hall.env")), "{paths:?}");
}

#[test]
fn feature_session_sandbox_grants_features_dir_and_promoted_repo_dirs() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000001").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();
    let sandbox = Sandbox::from_writable_set(
        &set,
        &layout,
        Some(&feature),
        Provider::ClaudeCode,
        &session_id,
    )
    .unwrap();

    let roots = sandbox.roots();

    // Feature sessions grant the canonical .ivar/features directory for mid-session child creation
    let canonical_features_dir = layout.features_dir().canonicalize_utf8().unwrap();
    assert!(
        roots.contains(&canonical_features_dir),
        "sandbox roots must contain canonical features_dir {canonical_features_dir}: {roots:?}"
    );

    // Feature sessions grant the canonical repo directory under .ivar/repos/<repo> for promoted repos
    let canonical_api_repo_dir = layout
        .repo_dir(&RepoName::new("api").unwrap())
        .canonicalize_utf8()
        .unwrap();
    assert!(
        roots.contains(&canonical_api_repo_dir),
        "sandbox roots must contain canonical repo_dir {canonical_api_repo_dir}: {roots:?}"
    );
}

#[test]
fn discovery_session_sandbox_does_not_grant_features_dir_or_repo_dirs() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000002").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let sandbox =
        Sandbox::from_writable_set(&set, &layout, None, Provider::ClaudeCode, &session_id).unwrap();

    let roots = sandbox.roots();

    let features_dir = layout.features_dir().canonicalize_utf8().unwrap();
    assert!(
        !roots.contains(&features_dir),
        "discovery sandbox roots must not contain features_dir: {roots:?}"
    );

    let api_repo_dir = layout
        .repo_dir(&RepoName::new("api").unwrap())
        .canonicalize_utf8()
        .unwrap();
    assert!(
        !roots.contains(&api_repo_dir),
        "discovery sandbox roots must not contain repo_dir: {roots:?}"
    );
}
