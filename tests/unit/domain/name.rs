#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use rstest::rstest;

use super::*;

// -- segment types: HallName, RepoName, FeatureName -------

#[rstest]
#[case::plain("api")]
#[case::kebab("api-gateway")]
#[case::underscored("api_gateway")]
#[case::digits("v2")]
#[case::dot_in_middle("api.v2")]
fn segment_accepts_well_formed_names(#[case] value: &str) {
    assert_eq!(RepoName::new(value).unwrap().as_str(), value);
}

#[rstest]
#[case::empty("", InvalidName::Empty)]
#[case::whitespace_only("   ", InvalidName::Empty)]
#[case::tab_only("\t", InvalidName::Empty)]
#[case::dot(".", InvalidName::Traversal)]
#[case::dotdot("..", InvalidName::Traversal)]
#[case::embedded_slash("api/gateway", InvalidName::NotASegment)]
#[case::embedded_backslash("api\\gateway", InvalidName::NotASegment)]
#[case::leading_dot(".hidden", InvalidName::Hidden)]
#[case::leading_dotdot("..hidden", InvalidName::Hidden)]
#[case::leading_whitespace(" api", InvalidName::Whitespace)]
#[case::trailing_whitespace("api ", InvalidName::Whitespace)]
#[case::control_character("api\u{7}gateway", InvalidName::ControlCharacter)]
#[case::nul("api\0gateway", InvalidName::ControlCharacter)]
#[case::newline("api\ngateway", InvalidName::ControlCharacter)]
fn segment_rejects_bad_names(#[case] value: &str, #[case] expected: InvalidName) {
    assert_eq!(RepoName::new(value).unwrap_err(), expected);
}

#[test]
fn segment_rules_apply_to_every_segment_type() {
    assert!(HallName::new("../etc").is_err());
    assert!(FeatureName::new("../etc").is_err());
}

// -- BranchName -----------------------------------------------------------

#[rstest]
#[case::plain("main")]
#[case::nested("feat/auth-v2")]
#[case::deeply_nested("release/2024/q1")]
#[case::numeric_leaf("feat/123")]
#[case::minor_version("release/2.0")]
#[case::not_a_dot_lock_suffix("fix/a.lockfile")]
#[case::patch_version("v1.0.0")]
fn branch_accepts_well_formed_names(#[case] value: &str) {
    assert_eq!(BranchName::new(value).unwrap().as_str(), value);
}

#[rstest]
#[case::empty("", InvalidName::Empty)]
#[case::whitespace_only("   ", InvalidName::Empty)]
#[case::dot(".", InvalidName::Traversal)]
#[case::dotdot("..", InvalidName::Traversal)]
#[case::dotdot_in_segment("feat/../etc", InvalidName::Traversal)]
#[case::embedded_backslash("feat\\auth", InvalidName::ForbiddenCharacter('\\'))]
#[case::leading_whitespace(" main", InvalidName::Whitespace)]
#[case::trailing_whitespace("main ", InvalidName::Whitespace)]
#[case::control_character("feat\u{7}", InvalidName::ControlCharacter)]
#[case::nul("feat\0auth", InvalidName::ControlCharacter)]
#[case::leading_slash("/main", InvalidName::LeadingOrTrailingSlash)]
#[case::trailing_slash("main/", InvalidName::LeadingOrTrailingSlash)]
#[case::double_slash("feat//auth", InvalidName::DoubleSlash)]
#[case::leading_dash("-main", InvalidName::LeadingDash)]
#[case::trailing_dot_lock("main.lock", InvalidName::TrailingDotLock)]
#[case::trailing_dot_lock_mid_component("feat/x.lock/y", InvalidName::TrailingDotLock)]
#[case::leading_dot_component("feat/.hidden", InvalidName::Hidden)]
#[case::leading_dot_first_component(".hidden/feat", InvalidName::Hidden)]
#[case::trailing_dot("feat/x.", InvalidName::TrailingDot)]
#[case::tilde("feat~1", InvalidName::ForbiddenCharacter('~'))]
#[case::caret("feat^1", InvalidName::ForbiddenCharacter('^'))]
#[case::colon("feat:1", InvalidName::ForbiddenCharacter(':'))]
#[case::question("feat?", InvalidName::ForbiddenCharacter('?'))]
#[case::asterisk("feat*", InvalidName::ForbiddenCharacter('*'))]
#[case::open_bracket("feat[1]", InvalidName::ForbiddenCharacter('['))]
#[case::space("feat auth", InvalidName::ForbiddenCharacter(' '))]
#[case::bare_at("@", InvalidName::ReservedByGit)]
#[case::at_brace("feat/@{1}", InvalidName::RevisionSyntax)]
#[case::at_brace_upstream("@{upstream}", InvalidName::RevisionSyntax)]
fn branch_rejects_bad_names(#[case] value: &str, #[case] expected: InvalidName) {
    assert_eq!(BranchName::new(value).unwrap_err(), expected);
}

// -- SessionId --------------------------------------------------------------

#[test]
fn session_id_accepts_a_valid_uuid() {
    let raw = "2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c";
    assert_eq!(SessionId::new(raw).unwrap().as_str(), raw);
}

#[rstest]
#[case::empty("")]
#[case::whitespace_only("   ")]
#[case::dot(".")]
#[case::dotdot("..")]
#[case::embedded_slash("2c6e6f1e/2d8a")]
#[case::embedded_backslash("2c6e6f1e\\2d8a")]
#[case::leading_dot(".2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c")]
#[case::leading_whitespace(" 2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c")]
#[case::trailing_whitespace("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c ")]
#[case::control_character("2c6e6f1e\u{7}2d8a")]
#[case::nul("2c6e6f1e\u{0}2d8a")]
#[case::not_a_uuid_at_all("not-a-uuid")]
fn session_id_rejects_non_uuids(#[case] value: &str) {
    assert_eq!(SessionId::new(value).unwrap_err(), InvalidName::NotUuid);
}

// -- serde: Serialize is a bare string, Deserialize routes through `new` --

#[test]
fn serialize_is_a_bare_string_not_a_wrapped_object() {
    let repo = RepoName::new("api").unwrap();
    assert_eq!(serde_json::to_string(&repo).unwrap(), r#""api""#);
}

#[test]
fn deserializing_a_well_formed_string_round_trips() {
    let repo: RepoName = serde_json::from_str(r#""api""#).unwrap();
    assert_eq!(repo.as_str(), "api");
}

/// The point of the module. A derived `Deserialize` would accept this
/// silently; a hand-edited `ivar.json` could then smuggle `../` straight
/// past the type system and into a path join.
#[test]
fn deserializing_a_traversal_string_fails() {
    let result: Result<RepoName, _> = serde_json::from_str(r#""../../etc""#);
    assert!(result.is_err());

    let result: Result<BranchName, _> = serde_json::from_str(r#""../../etc""#);
    assert!(result.is_err());
}

#[test]
fn deserializing_an_empty_string_fails() {
    let result: Result<FeatureName, _> = serde_json::from_str(r#""""#);
    assert!(result.is_err());
}

// -- InvalidName -> Failure ---------------------------------------------

#[test]
fn invalid_name_converts_to_a_blocked_failure_with_a_specific_fix() {
    let failure: Failure = InvalidName::Traversal.into();
    assert_eq!(failure.code, "name.traversal");
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(failure.fix_actions[0].safe);
}

#[test]
fn every_variant_has_its_own_code_and_fix_action() {
    let variants = [
        InvalidName::Empty,
        InvalidName::Whitespace,
        InvalidName::NotASegment,
        InvalidName::Traversal,
        InvalidName::Hidden,
        InvalidName::ControlCharacter,
        InvalidName::LeadingOrTrailingSlash,
        InvalidName::DoubleSlash,
        InvalidName::LeadingDash,
        InvalidName::TrailingDotLock,
        InvalidName::TrailingDot,
        InvalidName::ForbiddenCharacter('~'),
        InvalidName::ReservedByGit,
        InvalidName::RevisionSyntax,
        InvalidName::NotUuid,
    ];

    let mut codes: Vec<&'static str> = variants.iter().map(InvalidName::code).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(
        codes.len(),
        variants.len(),
        "every variant needs its own code"
    );

    for variant in &variants {
        let fix = variant.fix_action();
        assert!(!fix.what.is_empty());
    }
}
