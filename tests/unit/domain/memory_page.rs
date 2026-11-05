use super::*;

#[test]
fn kinds_map_to_tiers() {
    assert_eq!(PageKind::Product.tier(), Tier::Semantic);
    assert_eq!(PageKind::Procedure.tier(), Tier::Procedural);
    assert_eq!(PageKind::Session.tier(), Tier::Episodic);
    assert!(PageKind::Update.is_durable() && !PageKind::Session.is_durable());
    assert_eq!(PageKind::parse("decision"), Some(PageKind::Decision));
    assert_eq!(PageKind::parse("handoff"), None);
}
