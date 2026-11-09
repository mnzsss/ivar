//! Waiting for name resolution before a timer run.
//!
//! A `catchup` run fires as soon as the systemd user manager starts, often
//! before the network is up, and a user unit cannot order itself after
//! `network-online.target`, which lives in the system manager.

use std::net::ToSocketAddrs;
use std::time::Duration;

pub(super) const BUDGET: Duration = Duration::from_secs(120);
pub(super) const INTERVAL: Duration = Duration::from_secs(2);

pub(crate) fn resolves(host: &str) -> bool {
    (host, 443)
        .to_socket_addrs()
        .is_ok_and(|mut addrs| addrs.next().is_some())
}

/// Polls until `host` resolves or `budget` runs out. A run that starts without
/// a network fails on its own, with the tool's error in its receipt.
pub(super) fn wait_for_dns(
    host: &str,
    resolves: &dyn Fn(&str) -> bool,
    sleep: &dyn Fn(Duration),
    budget: Duration,
    interval: Duration,
) -> bool {
    let mut waited = Duration::ZERO;
    loop {
        if resolves(host) {
            return true;
        }
        if waited >= budget {
            return false;
        }
        sleep(interval);
        waited += interval;
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/agent/run/network.rs"]
mod tests;
