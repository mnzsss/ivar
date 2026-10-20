---
schedule: "30 6 * * 1"
tool: codex
model: my-omniroute-combo
network: [host.docker.internal]
secrets: [OPENAI_API_KEY, OPENAI_BASE_URL]
timeout: 30m
---
Audit the dependency manifests in every repo under /workspace (Cargo.toml,
package.json, lockfiles). Report dependencies more than one major version
behind, and any with a known security advisory you can identify from the
lockfile alone. Do not change any file.
