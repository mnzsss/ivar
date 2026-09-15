#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::test_support::hall_root;

#[test]
fn review_comments_round_trip_through_the_feature_review_file() {
    let (_guard, root) = hall_root();
    let layout = Layout::at(root);
    let name = FeatureName::new("checkout").unwrap();
    assert!(
        ReviewComments::read(&layout, &name)
            .unwrap()
            .comments
            .is_empty()
    );

    let comments = ReviewComments {
        next_id: 2,
        comments: vec![ReviewComment {
            id: "c1".to_owned(),
            repo: RepoName::new("api").unwrap(),
            file: "src/lib.rs".to_owned(),
            line_start: 3,
            line_end: 5,
            body: "rename this".to_owned(),
            status: CommentStatus::Open,
            created_at: 10,
            resolved_at: None,
        }],
    };
    comments.write(&layout, &name).unwrap();

    assert!(
        layout
            .review_comments_file(&name)
            .ends_with("features/checkout/review/comments.json")
    );
    assert_eq!(ReviewComments::read(&layout, &name).unwrap(), comments);
}

#[test]
fn ids_start_at_one_even_for_files_written_with_next_id_zero() {
    assert_eq!(ReviewComments::default().next_id, 1);
    let legacy: ReviewComments = serde_json::from_str(r#"{"next_id":0,"comments":[]}"#).unwrap();
    assert_eq!(legacy.next_id, 1);
}
