#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

// -- needs_preregistration: the allowlist table --------------------------

#[test]
fn figma_host_needs_preregistration() {
    assert!(needs_preregistration("mcp.figma.com"));
}

#[test]
fn other_hosts_do_not() {
    assert!(!needs_preregistration("mcp.linear.app"));
    assert!(!needs_preregistration("api.github.com"));
    assert!(!needs_preregistration(""));
}

// -- registration_failure: the 403-vs-generic split ----------------------

#[test]
fn a_403_names_the_allowlist_not_the_caller() {
    let failure = registration_failure(403, "forbidden".to_owned());
    assert_eq!(failure.code, "figma.register_client_http");
    let fix = failure.fix_actions.first().expect("a fix action");
    assert!(
        fix.what.contains("allowlist"),
        "fix should name the allowlist: {}",
        fix.what
    );
}

#[test]
fn other_statuses_get_a_generic_failure_with_no_fix() {
    let failure = registration_failure(500, "server error".to_owned());
    assert_eq!(failure.code, "figma.register_client_http");
    assert!(failure.fix_actions.is_empty());
    assert_eq!(failure.actual.as_deref(), Some("server error"));
}

// -- real 403, over the network --------------------------------------------
//
// Everything above tests pure helpers and cannot catch a regression in which
// *branch the transport takes* — ureq 3.x defaults to turning any non-2xx
// status into a transport error before application code sees it, which
// would make the 403 branch in `register_client_as` dead code even though
// `registration_failure` itself is correct. Only a real request exercises
// that. `client_name: "opencode"` is confirmed to 403 against Figma's
// registration endpoint (it's the name OpenCode's own dynamic registration
// sends, and it is not on the allowlist), so this hits the network for
// real and is `#[ignore]`d to keep CI offline. Run with
// `cargo test -- --ignored`.
#[test]
#[ignore = "hits the real Figma registration endpoint over the network"]
fn a_real_403_names_the_allowlist() {
    let error = register_client_as("http://127.0.0.1:19876/callback", "opencode").unwrap_err();
    assert_eq!(error.code, "figma.register_client_http");
    let fix = error.fix_actions.first().expect("a fix action on a 403");
    assert!(
        fix.what.contains("allowlist"),
        "fix should name the allowlist: {}",
        fix.what
    );
}
