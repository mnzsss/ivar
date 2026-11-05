use super::*;
use crate::domain::memory_page::Tier;

#[test]
fn user_text_never_becomes_fts_syntax() {
    assert_eq!(
        fts_expression("worktree teardown").as_deref(),
        Some("\"worktree\" OR \"teardown\"")
    );
    assert_eq!(
        fts_expression("AND NEAR( * \"quoted\"").as_deref(),
        Some("\"AND\" OR \"NEAR(\" OR \"*\" OR \"quoted\"")
    );
    assert_eq!(fts_expression("   "), None);
}

#[test]
fn a_quote_only_input_is_blank() {
    assert_eq!(fts_expression("\""), None);
}

#[test]
fn the_tier_multiplier_is_bounded_and_ordered() {
    assert!(tier_multiplier(Tier::Semantic) >= tier_multiplier(Tier::Procedural));
    assert!(tier_multiplier(Tier::Procedural) > tier_multiplier(Tier::Episodic));
    for tier in [
        Tier::Semantic,
        Tier::Procedural,
        Tier::Episodic,
        Tier::Working,
    ] {
        assert!((0.8..=1.2).contains(&tier_multiplier(tier)));
    }
}
