# Scheduled agents

An agent is one Markdown file in `agents/` at the hall root. Its frontmatter
says when and how it runs; its body is the prompt. systemd fires each run;
ivar runs no process of its own between runs. Linux with systemd only.

## An agent file

`agents/triage.md`:

    ---
    schedule: "0 7 * * 1-5"
    tool: claude
    model: sonnet
    network: [api.anthropic.com]
    secrets: [ANTHROPIC_API_KEY]
    notify: https://hooks.example.com/ivar
    timeout: 20m
    workdir: app
    ---
    Read the open issues in app and write a triage summary.

## Fields

| Field | Default | Rule |
| --- | --- | --- |
| `schedule` | required | 5-field cron, or `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`; refused when day-of-month and day-of-week are both set |
| `tool` | — | `codex`, `claude` or `opencode`; required unless `image` is set |
| `model` | tool default | passed to the tool unchanged |
| `image` + `command` | — | run your own image and argv instead of `tool` |
| `access` | `read` | `read`: repos mounted read-only, the run produces a report. `feature`: only the `agent-<name>` feature worktree is writable |
| `runtime` | `docker` | `docker`: the container below. `host`: the installed `tool` under your own login, confined by Landlock; needs `tool`, refuses `image`, `command` and `network` |
| `workdir` | `/workspace` | relative path under the hall's repos, first segment is the repo; required for `access: feature` |
| `write` | `[]` | `runtime: host` only. Extra writable paths: absolute (`/srv/out`), home-relative (`~/.cache/x`) or hall-relative (`brand/metricas`); no `..`. Each must exist when the run starts |
| `env` | `{}` | `runtime: host` only. Literal variables for the tool's process, such as `{TMPDIR: /tmp, XDG_CURRENT_DESKTOP: KDE}` (with `/tmp` also in `write`: the run writes only its scratch, its state directory and the `write` paths). Names match `^[A-Z_][A-Z0-9_]*$`; `PATH`, `HOME`, any `GIT_*`, the state variables of every tool (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`) whatever `tool` is set, `OPENCODE_CONFIG_CONTENT` and names in `secrets` are refused. Values stay in the agent file as written and are not redacted: never put a secret here |
| `network` | `[]` | bare hostnames the egress proxy allows; everything else fails |
| `secrets` | `[]` | names (`^[A-Z][A-Z0-9_]*$`) read from `.ivar/secrets/agents.env` |
| `notify` | — | `http(s)://` URL that receives a JSON run summary, or `telegram:<chat_id>` to message a Telegram chat (see [Telegram](#telegram)) |
| `timeout` | `30m` | `90s`, `45m`, `2h`; at most `24h` |
| `catchup` | `false` | `true`: when the machine was off or asleep at the scheduled time, run once as soon as the timer is active again, however many runs were missed. Sets `Persistent=true` on the timer; run `ivar agent sync` after changing it |

## Commands

| Command | Does |
| --- | --- |
| `ivar agent sync` | installs, updates and removes the systemd user timers for `agents/*.md` |
| `ivar agent status` | schedule, next run, last outcome and timer state per agent |
| `ivar agent run <name>` | runs now, through the same path the timer uses |
| `ivar agent logs <name> [--run <id>]` | output of the latest or given run |
| `ivar agent feedback <run-id> --rating up [--comment <text>]` | records your verdict on the run; `--rating` is `up` or `down` |
| `ivar agent notify telegram setup` | asks for the bot token, finds the chat, sends a test message and sets `notify` on the agents you pick |

Every command takes `--json`.

## Telegram

`notify: telegram:<chat_id>` sends each run's outcome and the last 2000
characters of its output to a Telegram chat, fewer when the message would
pass Telegram's 4096-unit limit. `<chat_id>` is a number
(`123456`, `-1001234567890` for a group) or a public channel's
`@username`; to post in a channel, the bot must be one of its admins.

Create a bot with @BotFather, then run `ivar agent notify telegram setup`.
It asks for the token without echoing it, checks it with Telegram, saves
it in `.ivar/secrets/agents.env` as `TELEGRAM_BOT_TOKEN`, lists the chats
that have messaged the bot, sends a test message, and offers to set
`notify` on your agents; an agent file it cannot update is left as it
was and listed with the reason. It needs a terminal; without one, put the token
in `agents.env` yourself and write the `notify` line by hand.

ivar reads the token when the run ends; the agent does not see it unless
it lists it in `secrets`. A failed send is recorded in the receipt's
`notify_error`, with the token replaced by `[redacted]`, and never changes
the run's outcome. Changing `notify` needs no `ivar agent sync`.

## What a run can reach

The container runs as uid 1000 with every capability dropped, no Docker
socket, no home directory and no git credentials. Its only network is an
internal Docker network whose one exit is a tinyproxy that allows the
domains in `network`. To reach a gateway on the host, list
`host.docker.internal`.

## Runtime: host

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
worktree. Its environment starts empty: `PATH`, `HOME`, locale, the
variables that point the tool at its private state, the secrets it lists,
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
can replace `TMPDIR` or a kept variable such as `LANG`, but never one that
confines the tool. Use it for what a desktop program needs from your
session. Chrome decrypts saved logins only when it reaches your keyring,
which on KDE takes `XDG_CURRENT_DESKTOP`, `DESKTOP_SESSION`,
`KDE_SESSION_VERSION`, `DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR`.
The D-Bus address hands the run your keyring and every other session
service, so list it only for an agent you trust with them.
`SSH_AUTH_SOCK` likewise hands the run your ssh-agent, so it could push
over SSH.

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

## Outcomes

`succeeded`, `failed`, `blocked` (the tool asked for input; the run is
stopped at once), `skipped` (the previous run was still going),
`timed_out`. Receipts live in `.ivar/agents/runs/<agent>/<run-id>.json`.

## `access: feature`

The run writes into feature `agent-<name>`. ivar commits on the host after
the container exits and records a `deliver` preview fingerprint in the
receipt. Nothing is pushed: run `ivar feature deliver agent-<name>
--fingerprint <fp>` yourself to open the PR.

Pushing is meant to happen on the host, through `deliver`. Do not give a
feature agent a git host in `network` together with a token in `secrets`:
that pair lets it push from inside the container.

## Secrets

`.ivar/secrets/agents.env` holds `NAME=value` lines and must be mode 600.
Only the names an agent lists are passed in, and their values are removed
from receipts, logs and notifications.

## Requirements

`ivar doctor` reports each missing piece and its fix: a systemd user
instance, linger (`loginctl enable-linger $USER`), the Docker client, a
reachable daemon, and a warning when Docker runs as root.

For `runtime: host` agents it checks instead that the tool is on `PATH`
(`agent.tool_missing`) and that the kernel has Landlock
(`agent.landlock_unavailable`); without Landlock a host run is refused
and its receipt is `failed`. Docker is not checked when no agent uses it.

An agent file that fails to load when its timer fires writes no receipt;
its error is only in the systemd journal: `journalctl --user -u <unit>`.
