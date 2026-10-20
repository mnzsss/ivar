//! The container images a scheduled agent runs in, pinned to this binary's release.

pub const BASE_IMAGE: &str = concat!("ghcr.io/mnzsss/ivar-agent:", env!("CARGO_PKG_VERSION"));
pub const PROXY_IMAGE: &str = concat!(
    "ghcr.io/mnzsss/ivar-agent-proxy:",
    env!("CARGO_PKG_VERSION")
);
