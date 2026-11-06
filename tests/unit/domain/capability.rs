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
            memory_review,
            memory_brief: _,
            mcp_client_name,
        } = provider_support(provider);
        assert!(!mcp_client_name.is_empty(), "{provider}");
        assert_eq!(cleanup_command, Support::Supported, "{provider}");
        assert_eq!(plan_memory_step, Support::Supported, "{provider}");
        assert_eq!(memory_mcp, Support::Supported, "{provider}");
        assert_eq!(planning_memory, Support::Supported, "{provider}");
        assert_eq!(memory_misses, Support::Supported, "{provider}");
        assert_eq!(memory_review, Support::Supported, "{provider}");
    }
}

#[test]
fn only_opencode_lacks_a_brief_because_its_plugins_cannot_inject_context() {
    for provider in [Provider::ClaudeCode, Provider::T3, Provider::Omp] {
        assert_eq!(
            provider_support(provider).memory_brief,
            Support::Supported,
            "{provider}"
        );
    }
    assert!(matches!(
        provider_support(Provider::OpenCode).memory_brief,
        Support::NotSupported(reason) if reason.contains("cannot inject context")
    ));
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

#[test]
fn client_names_resolve_back_to_a_provider() {
    assert_eq!(
        provider_for_client_name("claude-code"),
        Some(Provider::ClaudeCode)
    );
    assert_eq!(
        provider_for_client_name("opencode"),
        Some(Provider::OpenCode)
    );
    assert_eq!(
        provider_for_client_name("omp-coding-agent"),
        Some(Provider::Omp)
    );
    assert_eq!(
        provider_support(Provider::T3).mcp_client_name,
        "claude-code"
    );
    assert_eq!(provider_for_client_name("zed"), None);
}

#[test]
fn every_selectable_client_name_resolves_to_a_provider_with_the_same_name() {
    for provider in Provider::SELECTABLE {
        let name = provider_support(provider).mcp_client_name;
        assert_eq!(
            provider_for_client_name(name)
                .map(|resolved| provider_support(resolved).mcp_client_name),
            Some(name),
            "{provider}"
        );
    }
}
