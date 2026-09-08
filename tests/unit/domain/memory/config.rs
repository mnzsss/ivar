#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::memory::{MemoryConfig, MemoryScope, ScopeName};

#[test]
fn scope_name_accepts_valid_kebab_and_rejects_separators_or_traversal() {
    assert!(ScopeName::new("product").is_ok());
    assert!(ScopeName::new("team-conventions").is_ok());
    assert!(ScopeName::new("").is_err());
    assert!(ScopeName::new("..").is_err());
    assert!(ScopeName::new("a/b").is_err());
    assert!(ScopeName::new("Invalid Name").is_err());
}

#[test]
fn memory_config_validates_unique_scopes_and_positive_budgets() {
    let scope1 = MemoryScope {
        id: ScopeName::new("product").unwrap(),
        purpose: "Product specifications".into(),
        budget: 4000,
        stable_topics: vec!["roadmap".into()],
    };
    let scope2 = MemoryScope {
        id: ScopeName::new("product").unwrap(),
        purpose: "Duplicate scope".into(),
        budget: 2000,
        stable_topics: vec![],
    };

    let config = MemoryConfig {
        scopes: vec![scope1, scope2],
    };
    assert!(config.validate().is_err());
}
