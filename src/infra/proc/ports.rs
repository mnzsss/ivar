//! Linux `/proc` port attribution: which ports a process (or a program) is
//! listening on.
//!
//! The one Linux-specific capability in this crate, isolated here so the
//! subprocess module stays portable. Both functions are best-effort: a missing
//! `/proc` (non-Linux) or an unreadable entry yields an empty list, never an
//! error — absence is a warning, not a failure.

/// Discover the listening ports of a process by PID, reading `/proc/net/tcp`
/// and `/proc/net/tcp6`.
///
/// Returns an empty list when the process has no open sockets or when we are on
/// a non-Linux platform (the `/proc` filesystem is absent), never an error —
/// absence is a warning, not a failure.
pub fn find_listening_ports(pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();

    for path in [
        format!("/proc/{pid}/net/tcp"),
        format!("/proc/{pid}/net/tcp6"),
    ] {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };

        for line in contents.lines().skip(1) {
            // /proc/net/tcp layout:
            //   sl  local_address  rem_address  st  ...
            // local_address is "IP:PORT" where IP is little-endian hex.
            let parts: Vec<&str> = line.split_whitespace().collect();
            let Some(local_addr) = parts.get(1) else {
                continue;
            };

            let Some((_ip, port_hex)) = local_addr.rsplit_once(':') else {
                continue;
            };

            if let Ok(port) = u16::from_str_radix(port_hex, 16) {
                ports.push(port);
            }
        }
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Discover the listening ports of every process whose command line mentions
/// `program` — the ticket-22 primitive: "the port a repo's process opened".
///
/// Walks `/proc/*/cmdline`, matches the program name, and unions each
/// process's listening ports via [`find_listening_ports`]. Best-effort: a
/// `/proc` entry that cannot be read is skipped, and a non-Linux host (no
/// `/proc`) yields an empty list — absence is a warning, not an error.
#[must_use]
pub fn find_ports_for_program(program: &str) -> Vec<u16> {
    let mut ports = Vec::new();

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return ports;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        // cmdline is NUL-separated; a space-free match on the joined string
        // is enough to catch `node dev.js`, `cargo run`, etc.
        if cmdline.replace('\0', " ").contains(program) {
            ports.extend(find_listening_ports(pid));
        }
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}
