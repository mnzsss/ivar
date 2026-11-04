use super::*;

/// Every selectable provider answers for every capability. The destructuring
/// names no field and has no `..`, so the first capability added to
/// `ProviderSupport` fails to compile here until this test asserts it.
#[test]
fn every_selectable_provider_declares_its_capabilities() {
    for provider in Provider::SELECTABLE {
        let ProviderSupport {} = provider_support(provider);
    }
}

#[test]
fn not_supported_carries_its_reason() {
    let support = Support::NotSupported("no session-end hook");

    assert_ne!(support, Support::Supported);
    assert!(matches!(support, Support::NotSupported(reason) if reason == "no session-end hook"));
}
