# Scheduled agents: host runtime

Part of [Scheduled agents](scheduled-agents.md).

`runtime: host` runs the `codex`, `claude` or `opencode` already installed
on your machine, with the login you use interactively, so no API key goes
in `secrets`. Each host agent gets a private copy of that login: before
every run, only the tool's login files are copied into
`.ivar/agents/state/<agent>/<tool>/`, and the tool keeps all its state
there. Your own tool config is not writable from the run, so an agent
cannot plant settings, hooks or plugins that run later outside the
sandbox. When the tool refreshes its token during a run, the refreshed
login is written back to your copy, and nothing else is.

Landlock limits its writes to a per-run scratch directory, the agent's
private state directory and, with `access: feature`, the `agent-<name>`
worktree. Its environment starts empty: `PATH`, `HOME`, locale, `TMPDIR`,
`TMP` and `TEMP` (all three point at the scratch directory), the variables that point the tool at its private state, the secrets it lists,
and git pointed at an empty config with no credential helper. No
credential variable is passed. When the run ends, its whole process group
is stopped.

`write` adds paths to that set. Each is resolved and canonicalized when the
run starts. A path that does not exist fails the run with
`agent.write_missing`. Landlock grants a whole subtree, so a path is refused
with `agent.write_refused` when it is, contains or sits inside a hall file
the session write guard protects (`.env`, `.mcp.json`, provider hook configs
such as `.claude/settings.json`), the hall's `.ivar/`, `.git/` or `agents/`,
or the tool's login directory, and when it contains the home, the hall or
the tool's login file. Every `write` is refused when the tool's login
directory holds the home or the hall (`CLAUDE_CONFIG_DIR=$HOME`): point it at
a dedicated directory. A protected path that does not resolve (a symlink
loop, or a dangling link through `..`) also refuses every `write` and is
named in the error. An empty path, `..` or `~user` is refused when the agent
file loads. Anything else you list becomes writable by the agent, including
config other programs read: keep the list to the directories the run must
produce.

`env` adds literal variables after ivar's own and before the secrets, so it
can replace `TMPDIR`, `TMP`, `TEMP` or a kept variable such as `LANG`, but never one that
confines the tool. Use it for what a desktop program needs from your
session. Chrome decrypts saved logins only when it reaches your keyring,
which on KDE takes `XDG_CURRENT_DESKTOP`, `DESKTOP_SESSION`,
`KDE_SESSION_VERSION`, `DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR`.
The D-Bus address hands the run your keyring and every other session
service, so list it only for an agent you trust with them.
`SSH_AUTH_SOCK` likewise hands the run your ssh-agent, so it could push
over SSH.

A claude host agent that runs Node, `tsx` or `mktemp` needs
`write: [/tmp]` and `env: {TMPDIR: /tmp}` in its frontmatter. The Bash tool
of claude may not inherit the scratch `TMPDIR`, and a program that falls
back to `/tmp` is denied by Landlock (`Permission denied` or `EACCES`),
which ends the run `blocked`. `ivar agent sync` warns
`agent.host_claude_tmp` for every claude host agent that has no writable
`/tmp`; the warning is only informative, so an agent that never creates
temporary files can ignore it. `/tmp` is shared with every process of your
user, so list it only for an agent you trust with that.

Chrome's own sandbox writes `/proc/self/uid_map`, so a host agent that
drives Chrome needs `/proc` in `write`. Landlock fixes each rule's path when
the run starts, and Chrome writes from child processes, so nothing narrower
works. `ivar agent sync` warns `agent.host_write_proc` for such an agent: it
can also change the `/proc` entries of your other processes that the kernel
lets your user write, while their memory stays behind ptrace rules.

What it does not isolate: **reads are ambient** and **the network is
open**. Anything your user can read is exposed, including SSH keys and
`~/.config/gh`, and so is any socket in `/run/user/<uid>` it can reach.
With the open network, an agent that can read an SSH key could push
with it. `network` is refused because nothing would enforce it. `ivar agent sync` prints
`agent.host_ambient_read` for every host agent. Use `runtime: docker`
for an agent you would not trust with your home directory.
