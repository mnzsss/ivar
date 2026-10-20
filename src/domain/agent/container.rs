//! The `docker run` argv for a sandboxed agent and its egress proxy. No I/O.

pub const PROXY_ALIAS: &str = "proxy";
pub const PROXY_PORT: u16 = 8888;
pub const AGENT_USER: &str = "1000:1000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub target: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerPlan {
    pub name: String,
    pub network: String,
    pub image: String,
    pub env: Vec<(String, String)>,
    pub env_file: String,
    pub mounts: Vec<Mount>,
    pub workdir: String,
    pub argv: Vec<String>,
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Never passes `-i`/`-t`, so the agent's stdin is `/dev/null`.
#[must_use]
pub fn agent_run_args(plan: &ContainerPlan) -> Vec<String> {
    let proxy = format!("http://{PROXY_ALIAS}:{PROXY_PORT}");
    let mut args = owned(&[
        "run",
        "--rm",
        "--name",
        &plan.name,
        "--network",
        &plan.network,
        "--user",
        AGENT_USER,
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--env-file",
        &plan.env_file,
    ]);
    for key in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"] {
        args.extend(["-e".to_owned(), format!("{key}={proxy}")]);
    }
    args.extend(owned(&[
        "-e",
        "HOME=/home/agent",
        "--tmpfs",
        "/home/agent",
        "--tmpfs",
        "/tmp",
    ]));
    for (key, value) in &plan.env {
        args.extend(["-e".to_owned(), format!("{key}={value}")]);
    }
    for m in &plan.mounts {
        let suffix = if m.read_only { ":ro" } else { "" };
        args.extend([
            "-v".to_owned(),
            format!("{}:{}{suffix}", m.source, m.target),
        ]);
    }
    args.extend(["-w".to_owned(), plan.workdir.clone(), plan.image.clone()]);
    args.extend(plan.argv.iter().cloned());
    args
}

#[must_use]
pub fn proxy_run_args(name: &str, network: &str, config_dir: &str, image: &str) -> Vec<String> {
    owned(&[
        "run",
        "-d",
        "--rm",
        "--name",
        name,
        "--network",
        network,
        "--network-alias",
        PROXY_ALIAS,
        "--add-host=host.docker.internal:host-gateway",
        "--user",
        "65534:65534",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "-v",
        &format!("{config_dir}:/etc/tinyproxy:ro"),
        image,
    ])
}

/// No `User`/`Group`: the container already runs as nobody, so tinyproxy must
/// not try to setuid, and port 8888 needs no capability.
#[must_use]
pub fn tinyproxy_conf() -> String {
    format!(
        "Port {PROXY_PORT}\nListen 0.0.0.0\nTimeout 600\n\
         ConnectPort 443\nConnectPort 80\nFilterDefaultDeny Yes\nFilterType ere\n\
         Filter \"/etc/tinyproxy/filter\"\nLogLevel Connect\n"
    )
}

#[must_use]
pub fn tinyproxy_filter(domains: &[String]) -> String {
    domains
        .iter()
        .map(|d| format!("^{}$\n", d.replace('.', "\\.")))
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/container.rs"]
mod tests;
