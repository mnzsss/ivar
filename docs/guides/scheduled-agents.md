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
| `schedule` | required, unless `at` is set | 5-field cron, or `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`; refused when day-of-month and day-of-week are both set |
| `at` | — | makes the file an [Agent Task](#agent-tasks): one run at an absolute time. Replaces `schedule` (refused together) and refuses `catchup` |
| `tool` | — | `codex`, `claude` or `opencode`; required unless `image` is set |
| `model` | tool default | passed to the tool unchanged |
| `image` + `command` | — | run your own image and argv instead of `tool` |
| `access` | `read` | `read`: repos mounted read-only, the run produces a report. `feature`: only the `agent-<name>` feature worktree is writable |
| `runtime` | `docker` | `docker`: the container below. `host`: the installed `tool` under your own login, confined by Landlock; needs `tool`, refuses `image`, `command` and `network` |
| `workdir` | `/workspace` | relative path under the hall's repos, first segment is the repo; required for `access: feature` |
| `write` | `[]` | `runtime: host` only. Extra writable paths: absolute (`/srv/out`), home-relative (`~/.cache/x`) or hall-relative (`brand/metricas`); no `..`. Each must exist when the run starts |
| `env` | `{}` | `runtime: host` only. Literal variables for the tool's process, such as `{TMPDIR: /tmp, XDG_CURRENT_DESKTOP: KDE}` (with `/tmp` also in `write`: the run writes only its scratch, its state directory and the `write` paths; a `TMPDIR`, `TMP` or `TEMP` pointing elsewhere makes `mktemp` and `os.tmpdir()` fail with `Permission denied`). Names match `^[A-Z_][A-Z0-9_]*$`; `PATH`, `HOME`, any `GIT_*`, the state variables of every tool (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`) whatever `tool` is set, `OPENCODE_CONFIG_CONTENT` and names in `secrets` are refused. Values stay in the agent file as written and are not redacted: never put a secret here |
| `network` | `[]` | bare hostnames the egress proxy allows; everything else fails |
| `secrets` | `[]` | names (`^[A-Z][A-Z0-9_]*$`) read from `.ivar/secrets/agents.env` |
| `notify` | — | `http(s)://` URL that receives a JSON run summary, or `telegram:<chat_id>` to message a Telegram chat (see [Telegram](#telegram)) |
| `timeout` | `30m` | `90s`, `45m`, `2h`; at most `24h` |
| `catchup` | `false` | recurring agents only. `true`: when the machine was off or asleep at the scheduled time, run once as soon as the timer is active again, however many runs were missed. Sets `Persistent=true` on the timer; run `ivar agent sync` after changing it |

## Agent Tasks

An Agent Task is an agent file with `at:` instead of `schedule:`. It runs once,
from its timer, at an absolute time, and then moves to `agents/done/`. Use it
for a deferred check: "look at this again in two days". It is unrelated to the
tasks in a feature plan.

`agents/recheck-prices.md`:

    ---
    at: 2026-11-08T09:00
    tool: claude
    network: [api.anthropic.com]
    notify: telegram:-1001234567890
    ---
    Check whether the vendor changed the price of the Pro plan. Report only the change.

`at` is an ISO 8601 date and time with minute precision, with no seconds:

- `2026-11-08T09:00` is local time on the machine that runs the timer.
- `2026-11-08T09:00-03:00` (or `…Z`) carries its own offset, so it names the same
  instant everywhere.
- Relative values such as `2d` are refused: work out the date and write it.

Every other field works as in a recurring agent (`access`, `runtime`, `network`,
`secrets`, `notify`, `timeout`, `workdir`). `schedule` and `catchup` cannot be
combined with `at`.

### What `ivar agent sync` does

- It writes a one-shot systemd timer with `OnCalendar=` set to the resolved time and
  `Persistent=true` always, so a machine that was off or asleep at that time runs the
  task when it comes back; this is the recovery after downtime, and needs no `catchup`.
- It ignores `agents/done/` as a source of agents.
- It removes the timer and service of every concluded task, which is how a finished
  task's unit goes away.
- When `at` is already in the past, it starts the task right away as a timer run and
  says so (`fired now`). It does the same for a task that is still pending after its
  timer elapsed, for example one that ended `skipped`. A time that falls in a clock
  change and that `date` cannot place is not fired by guesswork; fix the `at`.

### Rehearsal versus timer run

Only a run fired by the timer concludes a task, whatever its outcome except `skipped`:
`succeeded`, `failed`, `blocked` and `timed_out` all conclude it. A `skipped` run (the
previous run was still going) never ran, so the task stays pending.

`ivar agent run <name>` is a rehearsal. It runs the same path, writes a receipt and
notifies, but the task stays pending and its timer is untouched. A rehearsal's receipt
carries `trigger: manual`.

There is no automatic retry. A task that `failed` is concluded; copy it back from
`agents/done/` to `agents/` with a new `at` to run it again.

### Conclusion

When a timer run concludes a task:

- The file moves to `agents/done/<name>.<at as YYYY-MM-DDTHHMM>.md`, for example
  `agents/done/recheck-prices.2026-11-08T0900.md`. If that name exists, the run id is
  appended (`<name>.<at>.<run-id>.md`). An existing file is never overwritten. The time
  in the name is the `at` as written, with no offset.
- The receipt and the `notify` message show the scheduled time and the real one
  (`scheduled 2026-11-08T09:00 · ran 2026-11-08T12:00:03…Z`). The receipt keeps the
  scheduled time in `scheduled_at`, and a webhook summary carries it too.
- An `agent_task.concluded` observation goes through the memory spool, like the
  delivery events, and reaches `memory.db` on the next drain. Its body holds the task
  `name`, `at`, `ran_at`, `outcome`, `run_id` and the `output_tail`. It is a plain
  event with no prompt origin, so no session treats it as something the user said.
- If the file cannot be moved, the receipt is still written and the command warns with
  `agent.task_not_concluded`; the task stays pending and no observation is recorded.

`ivar agent status` lists each pending task with its time and the resolved
`OnCalendar` value. Concluded tasks are not listed.

## Commands

| Command | Does |
| --- | --- |
| `ivar agent sync` | installs, updates and removes the systemd user timers for `agents/*.md` |
| `ivar agent status` | schedule (or the pending `at` of a task), next run, last outcome and timer state per agent |
| `ivar agent run <name>` | runs now, through the same path the timer uses; for an Agent Task it is a rehearsal and the task stays pending |
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

`succeeded`, `failed`, `blocked` (the tool asked for input, and the run is
stopped at once; or the run declared it, see below), `skipped` (the previous
run was still going), `timed_out`. Receipts live in
`.ivar/agents/runs/<agent>/<run-id>.json`. For an Agent Task, every outcome
except `skipped` concludes it when the timer fired the run; see
[Agent Tasks](#agent-tasks).

### Declaring an outcome

A run with `tool:` can report that it could not do its work. If the last
non-empty line of its output starts with `BLOCKED:` the receipt records
`blocked`; if it starts with `FAILED:` it records `failed`. The receipt's
`exit_code` stays what the tool returned. `ivar agent status`, `ivar agent
logs` and the notification show the declared outcome.

- The match is case-sensitive and on plain text: `**BLOCKED:**` or
  `blocked:` do not count. Only the last non-empty line counts, so a sentinel
  earlier in the output, or echoed back from the prompt, changes nothing.
- A timeout or a pending approval still wins over a declaration. A
  declaration wins over the exit code, so `BLOCKED:` with exit 1 is `blocked`.
- It does not stop the run; it is read after the process ends.
- Runs with `image:` and `command:` are not prompt-driven and ignore it.
- A prompt whose output happens to end in `BLOCKED:` or `FAILED:` is
  now recorded accordingly; add a closing line if that is not what you want.

Tell the agent in its prompt to finish with `BLOCKED: <reason>` or
`FAILED: <reason>` when it cannot do the work.

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

Each service pins the `PATH` that `ivar agent sync` ran with, because a
`catchup` run fires as soon as the systemd user manager starts, before your
login session has exported its own `PATH`. That early, the network may be
down too, so a timer run of `claude` or `codex` first waits up to two minutes
for the tool's API host to resolve; after that it starts anyway and records
whatever the tool reports. Run `ivar agent sync` again after
installing the tool somewhere new. Run it with the installed `ivar` too: the
service runs the binary that synced it, and one under a cargo `target/`
directory warns `agent.ivar_bin_in_build_dir`, since a rebuild or
`cargo clean` changes or removes it.

An agent file that fails to load when its timer fires writes no receipt;
its error is only in the systemd journal: `journalctl --user -u <unit>`. An
Agent Task in that state stays pending, and its timer has already elapsed:
fix the file and run `ivar agent sync`, which fires it.
