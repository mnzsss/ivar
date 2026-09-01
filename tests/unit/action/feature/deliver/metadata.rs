use super::fixture::*;
use super::*;
use crate::action::Ctx;
use crate::action::feature::deliver::metadata::resolve;
use crate::domain::name::{BranchName, FeatureName, RepoName};

fn setup_feature_with_promotions(repos: &[&str]) -> (tempfile::TempDir, Ctx, Feature) {
    let (guard, root) = hall_with_promoted(repos);
    let ctx = Ctx::new(root.clone());
    let mut feature = Feature::new(
        FeatureName::new("checkout").unwrap(),
        BranchName::new("checkout").unwrap(),
    );
    for r in repos {
        feature.promote(RepoName::new(*r).unwrap());
    }
    (guard, ctx, feature)
}

#[test]
fn resolve_empty_metadata_produces_absent_fields_for_all_promoted_repos() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api", "web"]);
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata::default(),
        repo_overrides: Vec::new(),
    };

    let resolved = resolve(&ctx, &feature, &input).unwrap();
    assert_eq!(resolved.len(), 2);
    assert_eq!(
        resolved.get(&RepoName::new("api").unwrap()),
        Some(&PullRequestMetadata::default())
    );
    assert_eq!(
        resolved.get(&RepoName::new("web").unwrap()),
        Some(&PullRequestMetadata::default())
    );
}

#[test]
fn resolve_global_metadata_applies_to_all_repos() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api", "web"]);
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: Some("feat: global title".to_owned()),
            body: Some("global body inline".to_owned()),
            draft: None,
        },
        repo_overrides: Vec::new(),
    };

    let resolved = resolve(&ctx, &feature, &input).unwrap();
    let expected = PullRequestMetadata {
        title: Some("feat: global title".to_owned()),
        body: Some("global body inline".to_owned()),
        draft: None,
    };
    assert_eq!(
        resolved.get(&RepoName::new("api").unwrap()),
        Some(&expected)
    );
    assert_eq!(
        resolved.get(&RepoName::new("web").unwrap()),
        Some(&expected)
    );
}

#[test]
fn resolve_field_wise_inheritance_and_repo_overrides() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api", "web"]);
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: Some("feat: global title".to_owned()),
            body: Some("global body inline".to_owned()),
            draft: None,
        },
        repo_overrides: vec![
            RepoMetadataOverride {
                repo: "api".to_owned(),
                metadata: PullRequestMetadata {
                    title: Some("feat(api): custom title".to_owned()),
                    body: None,
                    draft: None,
                },
            },
            RepoMetadataOverride {
                repo: "web".to_owned(),
                metadata: PullRequestMetadata {
                    title: None,
                    body: Some("custom web body".to_owned()),
                    draft: None,
                },
            },
        ],
    };

    let resolved = resolve(&ctx, &feature, &input).unwrap();
    assert_eq!(
        resolved.get(&RepoName::new("api").unwrap()),
        Some(&PullRequestMetadata {
            title: Some("feat(api): custom title".to_owned()),
            body: Some("global body inline".to_owned()),
            draft: None,
        })
    );
    assert_eq!(
        resolved.get(&RepoName::new("web").unwrap()),
        Some(&PullRequestMetadata {
            title: Some("feat: global title".to_owned()),
            body: Some("custom web body".to_owned()),
            draft: None,
        })
    );

    assert_eq!(
        resolved.get(&RepoName::new("web").unwrap()),
        Some(&PullRequestMetadata {
            title: Some("feat: global title".to_owned()),
            body: Some("custom web body".to_owned()),
            draft: None,
        })
    );
}

#[test]
fn resolve_inline_body_versus_file_body() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api"]);
    // Write a file in ctx.cwd
    let file_path = ctx.cwd.join("body.md");
    std::fs::write(&file_path, "Content from file\n").unwrap();

    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: None,
            body: Some("./body.md".to_owned()),
            draft: None,
        },
        repo_overrides: Vec::new(),
    };

    let resolved = resolve(&ctx, &feature, &input).unwrap();
    assert_eq!(
        resolved.get(&RepoName::new("api").unwrap()),
        Some(&PullRequestMetadata {
            title: None,
            body: Some("Content from file\n".to_owned()),
            draft: None,
        })
    );

    // Non-prefixed or non-extension bodies remain inline
    let input_inline = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: None,
            body: Some("body.md".to_owned()), // missing leading ./
            draft: None,
        },
        repo_overrides: Vec::new(),
    };
    let resolved_inline = resolve(&ctx, &feature, &input_inline).unwrap();
    assert_eq!(
        resolved_inline.get(&RepoName::new("api").unwrap()),
        Some(&PullRequestMetadata {
            title: None,
            body: Some("body.md".to_owned()),
            draft: None,
        })
    );
}

#[test]
fn resolve_rejects_metadata_in_land_mode() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api"]);
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: true,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: Some("title".to_owned()),
            body: None,
            draft: None,
        },
        repo_overrides: Vec::new(),
    };

    let failure = resolve(&ctx, &feature, &input).unwrap_err();
    assert_eq!(failure.code, "deliver.metadata_in_land_mode");
}

#[test]
fn resolve_rejects_duplicate_repository_group() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api"]);
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata::default(),
        repo_overrides: vec![
            RepoMetadataOverride {
                repo: "api".to_owned(),
                metadata: PullRequestMetadata::default(),
            },
            RepoMetadataOverride {
                repo: "api".to_owned(),
                metadata: PullRequestMetadata::default(),
            },
        ],
    };

    let failure = resolve(&ctx, &feature, &input).unwrap_err();
    assert_eq!(failure.code, "deliver.duplicate_repo_group");
    assert!(failure.actual.as_deref().unwrap().contains("api"));
}

#[test]
fn resolve_rejects_unpromoted_repository_group() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api"]);
    let input = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata::default(),
        repo_overrides: vec![RepoMetadataOverride {
            repo: "unpromoted".to_owned(),
            metadata: PullRequestMetadata::default(),
        }],
    };

    let failure = resolve(&ctx, &feature, &input).unwrap_err();
    assert_eq!(failure.code, "deliver.unpromoted_repo_override");
    assert!(failure.actual.as_deref().unwrap().contains("unpromoted"));
}

#[test]
fn resolve_rejects_missing_or_invalid_body_file() {
    let (_guard, ctx, feature) = setup_feature_with_promotions(&["api"]);
    let input_missing = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: None,
            body: Some("./nonexistent.txt".to_owned()),
            draft: None,
        },
        repo_overrides: Vec::new(),
    };

    let failure = resolve(&ctx, &feature, &input_missing).unwrap_err();
    assert_eq!(failure.code, "deliver.body_file_read_failed");

    // Invalid UTF-8
    let invalid_path = ctx.cwd.join("invalid.txt");
    std::fs::write(&invalid_path, [0xFF, 0xFE, 0xFD]).unwrap();
    let input_invalid = DeliverInput {
        feature: "checkout".to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata {
            title: None,
            body: Some("./invalid.txt".to_owned()),
            draft: None,
        },
        repo_overrides: Vec::new(),
    };
    let failure_utf8 = resolve(&ctx, &feature, &input_invalid).unwrap_err();
    assert_eq!(failure_utf8.code, "deliver.body_file_not_utf8");
}
