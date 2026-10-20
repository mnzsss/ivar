#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn images_follow_the_binarys_own_release_tag() {
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(BASE_IMAGE, format!("ghcr.io/mnzsss/ivar-agent:{version}"));
    assert_eq!(
        PROXY_IMAGE,
        format!("ghcr.io/mnzsss/ivar-agent-proxy:{version}")
    );
}

#[test]
fn both_dockerfiles_exist_and_run_unprivileged() {
    let root = env!("CARGO_MANIFEST_DIR");
    let agent = std::fs::read_to_string(format!("{root}/images/agent/Dockerfile")).unwrap();
    assert!(agent.contains("USER 1000:1000"));
    let proxy = std::fs::read_to_string(format!("{root}/images/proxy/Dockerfile")).unwrap();
    assert!(proxy.contains("USER 65534:65534"));
}
