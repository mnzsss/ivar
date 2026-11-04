use super::*;

#[test]
fn a_merged_pull_request_for_the_local_head_is_read_as_delivered() {
    let answer = read_forge_answer(&[pull_request("MERGED", Some("abc"))], "abc");

    assert_eq!(answer, ForgeDelivery::Merged);
}

#[test]
fn a_merged_pull_request_for_another_head_is_read_as_a_different_merge() {
    let answer = read_forge_answer(&[pull_request("MERGED", Some("other"))], "abc");

    assert_eq!(answer, ForgeDelivery::MergedOtherHead { number: 7 });
}

#[test]
fn an_open_pull_request_is_read_as_not_merged() {
    let answer = read_forge_answer(&[pull_request("OPEN", Some("abc"))], "abc");

    assert_eq!(
        answer,
        ForgeDelivery::NotMerged {
            number: 7,
            state: "OPEN".to_owned(),
        }
    );
}

#[test]
fn an_empty_pull_request_list_is_read_as_no_pull_request() {
    assert_eq!(read_forge_answer(&[], "abc"), ForgeDelivery::NoPullRequest);
}

#[test]
fn the_merged_pull_request_for_the_local_head_wins_over_an_earlier_record() {
    let answer = read_forge_answer(
        &[
            pull_request("CLOSED", Some("other")),
            pull_request("MERGED", Some("abc")),
        ],
        "abc",
    );

    assert_eq!(answer, ForgeDelivery::Merged);
}

#[test]
fn a_repo_with_nothing_ahead_never_asks_the_forge() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let layout = Layout::at(root.clone());
    let manifest = read_manifest(&layout).unwrap();
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let previewed = preview_cleanup(&git::System, &layout, &manifest, &feature, None).unwrap();

    assert!(previewed.preview.repos[0].is_delivered);
    assert!(
        !previewed.forge_consulted,
        "local git proved delivery, so no forge answer may be recorded"
    );
}

#[test]
fn forge_answer_text_does_not_move_the_cleanup_fingerprint() {
    let feature = FeatureName::new("checkout").unwrap();
    let branch = BranchName::new("checkout").unwrap();
    let fingerprint_with = |forge: ForgeDelivery| {
        let facts = CleanupFacts {
            repos: vec![CleanupRepoFacts {
                repo: RepoName::new("api").unwrap(),
                effective_base: Some(BranchName::new("main").unwrap()),
                feature_head: Some("abc".to_owned()),
                base_head: Some("def".to_owned()),
                local_branch_exists: true,
                worktree_exists: true,
                clone_exists: true,
                dirty_worktree: Some(false),
                unmerged_commits: Some(1),
                in_manifest: true,
                inspection_error: None,
                forge_delivery: Some(forge),
            }],
            live_sessions: Vec::new(),
            session_inspection_error: None,
        };
        let blockers = classify_cleanup(&facts).blockers;
        let repos: Vec<_> = facts.repos.iter().filter_map(cleanup_repo).collect();
        let fingerprint = fingerprint_for(&feature, &branch, &repos, &blockers, &[], &[]).unwrap();
        (blockers, fingerprint)
    };

    let (first_blockers, first) = fingerprint_with(ForgeDelivery::Unavailable {
        reason: "gh: request timed out after 30s".to_owned(),
    });
    let (second_blockers, second) = fingerprint_with(ForgeDelivery::Unavailable {
        reason: "gh: connection reset".to_owned(),
    });

    assert_ne!(first_blockers, second_blockers);
    assert_eq!(first, second);
}

#[test]
fn preview_and_cleanup_follow_a_worktree_whose_dir_differs_from_the_branch() {
    let (_guard, root) = hall_with_feature(&["api"], None);
    let bare = root.join(".ivar/repos/api/.bare");
    let moved = root.join(".ivar/repos/api/elsewhere");
    git::System
        .move_worktree(&bare, &root.join(".ivar/repos/api/checkout"), &moved)
        .unwrap();

    let preview = run_preview(&root);

    assert_eq!(preview.paths_to_remove[0], moved);
    assert!(preview.repos[0].worktree_exists);
}

#[test]
fn only_a_pull_request_merged_at_the_exact_head_is_a_merged_observation() {
    let prs = [
        pull_request("OPEN", Some("abc")),
        pull_request("MERGED", Some("other")),
        pull_request("MERGED", Some("abc")),
    ];
    let found = merged_pull_request(&prs, "abc").expect("the merged head matches");
    assert_eq!(found.head_oid.as_deref(), Some("abc"));
    assert!(merged_pull_request(&prs[..2], "abc").is_none());
}
