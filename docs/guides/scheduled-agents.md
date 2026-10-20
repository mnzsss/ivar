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
| `workdir` | `/workspace` | relative path under the hall's repos, first segment is the repo; required for `access: feature` |
| `network` | `[]` | bare hostnames the egress proxy allows; everything else fails |
| `secrets` | `[]` | names (`^[A-Z][A-Z0-9_]*$`) read from `.ivar/secrets/agents.env` |
| `notify` | — | `http(s)://` URL that receives a JSON run summary |
| `timeout` | `30m` | `90s`, `45m`, `2h`; at most `24h` |

## Commands

| Command | Does |
| --- | --- |
| `ivar agent sync` | installs, updates and removes the systemd user timers for `agents/*.md` |
| `ivar agent status` | schedule, next run, last outcome and timer state per agent |
| `ivar agent run <name>` | runs now, through the same path the timer uses |
| `ivar agent logs <name> [--run <id>]` | output of the latest or given run |
| `ivar agent feedback <run-id> --rating up [--comment <text>]` | records your verdict on the run; `--rating` is `up` or `down` |

Every command takes `--json`.

## What a run can reach

The container runs as uid 1000 with every capability dropped, no Docker
socket, no home directory and no git credentials. Its only network is an
internal Docker network whose one exit is a tinyproxy that allows the
domains in `network`. To reach a gateway on the host, list
`host.docker.internal`.

## Outcomes

`succeeded`, `failed`, `blocked` (the tool asked for input; the run is
stopped at once), `skipped` (the previous run was still going),
`timed_out`. Receipts live in `.ivar/agents/runs/<agent>/<run-id>.json`.

## `access: feature`

The run writes into feature `agent-<name>`. ivar commits on the host after
the container exits and records a `deliver` preview fingerprint in the
receipt. Nothing is pushed: run `ivar feature deliver agent-<name>
--fingerprint <fp>` yourself to open the PR.

## Secrets

`.ivar/secrets/agents.env` holds `NAME=value` lines and must be mode 600.
Only the names an agent lists are passed in, and their values are removed
from receipts, logs and notifications.

## Requirements

`ivar doctor` reports each missing piece and its fix: a systemd user
instance, linger (`loginctl enable-linger $USER`), the Docker client, a
reachable daemon, and a warning when Docker runs as root.
