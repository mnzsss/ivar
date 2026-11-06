use super::*;

/// Every selectable provider answers for every capability. The destructuring
/// names no field and has no `..`, so the first capability added to
/// `ProviderSupport` fails to compile here until this test asserts it.
#[test]
fn every_selectable_provider_declares_its_capabilities() {
    for provider in Provider::SELECTABLE {
        let ProviderSupport {
            session_started: _,
            session_prompted: _,
            session_tool: _,
            session_ended: _,
            cleanup_command,
            plan_memory_step,
            memory_mcp,
            planning_memory,
            memory_misses,
        } = provider_support(provider);
        assert_eq!(cleanup_command, Support::Supported, "{provider}");
        assert_eq!(plan_memory_step, Support::Supported, "{provider}");
        assert_eq!(memory_mcp, Support::Supported, "{provider}");
        assert_eq!(planning_memory, Support::Supported, "{provider}");
        assert_eq!(memory_misses, Support::Supported, "{provider}");
    }
}

#[test]
fn opencode_declares_no_session_end_because_idle_is_a_turn_boundary() {
    let support = provider_support(Provider::OpenCode);

    assert_eq!(support.session_started, Support::Supported);
    assert_eq!(support.session_prompted, Support::Supported);
    assert_eq!(support.session_tool, Support::Supported);
    assert!(
        matches!(support.session_ended, Support::NotSupported(reason) if reason.contains("session.idle"))
    );
}

#[test]
fn t3_declares_session_end_because_closing_a_thread_fires_it() {
    assert_eq!(
        provider_support(Provider::T3).session_ended,
        Support::Supported
    );
}

#[test]
fn not_supported_carries_its_reason() {
    let support = Support::NotSupported("no session-end hook");

    assert_ne!(support, Support::Supported);
    assert!(matches!(support, Support::NotSupported(reason) if reason == "no session-end hook"));
}
