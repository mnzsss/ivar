---
name: ivar-agent
description: Create a scheduled ivar agent or an Agent Task when the user asks for one, suggest one when the user states a deferred check or a rhythm in the current turn, and remove an agent when asked
---

# ivar-agent

A scheduled agent is one Markdown file, `agents/<name>.md` at the hall root.
Its frontmatter says when and how it runs, its body is the prompt, and systemd
fires each run. The field reference is `docs/guides/scheduled-agents.md` in the
`ivar` repo; `ivar agent --help` lists the commands.

## When to suggest an agent

Suggest one only from a live signal: something the user states in the current
turn. Repetition across turns or sessions is not a signal here; the
`ivar-automate` skill owns it.

- **Deferred check**: an explicit action, a target and a deadline ("look at the
  vendor price again in two days"). Resolve the deadline to an absolute date. If
  there is none, ask for it. This becomes an Agent Task proposal.
- **Declared rhythm**: "every morning", "weekly", "remember to check". This
  becomes a recurring agent proposal.

Suggest at the end of the turn, never in the middle of other work, and once per
signal. The answers are accept (with corrections) or decline. A decline, or no
answer, records nothing, and there is no defer.

An agent created from a live signal records no automation decision. Linking it
to a repeated request happens later, through the `ivar-automate` skill's "link
to existing" form.

## The proposal

Propose in this block, filled in, then ask the question. Create nothing
before the answer.

Agent Task:

    Agent Task · deferred check
    Name:    <name>
    When:    <weekday, date, time, time zone> (<relative distance>) · at: <ISO 8601>
    Does:    <what the run looks at and produces>
    Runs:    tool <tool> · runtime <runtime> · access <access>
    scripts: <path> (<approved|unreviewed|changed|missing>), or none
    notify:  <target, or none>

    Create it? You can correct date, action or notify. The file goes in agents/ and you commit it.

Recurring agent: the same block with the header `Scheduled agent · declared
rhythm`, and `When:` showing the `schedule:` written out with the time zone (for
example `weekdays at 07:00 America/Sao_Paulo · schedule: 0 7 * * 1-5`). Use the
same closing question, with "schedule" instead of "date".

Fill each `scripts:` state from `ivar script check <path>`: it exits 0 when the
script is approved and 3 when it is unreviewed (never approved, or changed since).

## When an agent is the wrong tool

- The work needs the user's decision halfway: an unattended run that asks for
  input ends as `blocked`, and so does one whose last line is
  `BLOCKED: <reason>`.
- The trigger is an event, not a time. Agents run on a cron schedule or once at
  an absolute time (`at:`); the nearest fit is a frequent schedule that checks
  and exits when nothing changed.
- The machine has no systemd user instance (Linux only).
- The work is a feature the user wants to plan and review: use a feature and the
  `ivar-plan` skill instead.

Tell the user why and offer the alternative.

## Create the agent

Settle these with the user, defaulting to the safest value and stating the default
you picked:

1. **Name**: the file stem, short, lowercase, hyphenated. It also names the
   `agent-<name>` feature, so it must be a valid feature name.
2. **Schedule**: a 5-field cron or `@hourly`, `@daily`, `@weekly`, `@monthly`,
   `@yearly`. Day-of-month and day-of-week together are refused. Confirm the
   time zone the user means. For an Agent Task, write `at:` instead of
   `schedule:`, as an absolute ISO 8601 date and time with minute precision
   (`2026-11-08T09:00`, or with an offset such as `-03:00`). A relative value
   such as `2d` is refused: work out the date. `at` cannot be combined with
   `schedule` or `catchup`.
3. **Tool and model**: `codex`, `claude` or `opencode`, with an optional `model`.
4. **Runtime**: `docker` (default) confines reads, writes and network. Choose
   `host` only if the user accepts that reads and the network are open; it needs
   `tool` and refuses `network`, `image` and `command`.
5. **Access**: `read` (default) produces a report. `feature` needs `workdir` and
   writes into the `agent-<name>` feature; pushing stays a manual
   `ivar feature deliver`.
6. **Network and secrets**: the smallest `network` host list that works. Secrets
   are names matching `^[A-Z][A-Z0-9_]*$`; tell the user to put the values in
   `.ivar/secrets/agents.env` (mode 600) themselves. Never ask for a value in
   the conversation and never write one into the agent file.
7. **Scripts**: when the prompt calls hall scripts (`ivar script list`), list
   each in `scripts:` as a hall-relative path under `.ivar/scripts/` or
   `.ivar/scripts-local/`, and show the user its approval state from
   `ivar script check <path>`. Check that every variable in the script's
   `# env:` header is in `secrets` or `env`; `ivar agent sync` warns with
   `agent.script_env_missing` for one that is in neither. An unreviewed script
   blocks the run (`agent.script_unreviewed`) until a human approves it with
   `ivar script approve <path>` in an interactive session. Never approve one
   yourself.
8. **Notify and timeout**: an `http(s)` URL for the run summary, or
   `telegram:<chat_id>` to message a Telegram chat. For Telegram, tell the
   user to put the bot token in `.ivar/secrets/agents.env` as
   `TELEGRAM_BOT_TOKEN` (never ask for it here) and set
   `notify: telegram:<chat_id>`, or to run `ivar agent notify telegram setup`
   in a terminal. A `timeout` when the default 30m is wrong.

Write the prompt for a reader with no context: it names the repo, says what to
look at, what to produce and where, and what "nothing to report" looks like. It
cannot ask questions. Tell it to end its output with a final line
`BLOCKED: <reason>` when it could not do the work, or `FAILED: <reason>` when
it hit an error; the run is then recorded as `blocked` or `failed` instead of
`succeeded`. The line must start with that plain, upper-case text (no markdown
such as `**BLOCKED:**`) and be the last non-empty line of the output.

Then:

1. Write `agents/<name>.md`. If the hall is a git repository, hall files are
   committed straight to the hall's `main`, with the `commit` skill if the user
   asked for a commit. If it is not, the file is only saved.
2. Ask before `ivar agent sync`: it installs systemd user timers on this machine.
3. Ask before `ivar agent run <name> --wait`: it runs the tool, which may cost
   money. Use it as the first check; it returns once the run ends, then read the
   result with `ivar agent logs <name>`. Without `--wait` the run goes to the
   background and the command returns at once.
4. Show `ivar agent status` so the user sees the next run.
5. If `sync` or the run reports a missing requirement, run `ivar doctor` and
   relay its fix. A file that fails to load when its timer fires leaves no
   receipt; the error is only in `journalctl --user -u <unit>`.

For an Agent Task, tell the user that:

- the task moves to `agents/done/` once its timer run concludes, whatever the
  outcome except `skipped`;
- `ivar agent run <name>` is only a rehearsal: it runs and notifies, but the task
  stays pending and its timer is untouched;
- `ivar agent sync` is what installs the timer, so ask before it as above.

When the `ivar-automate` skill accepts the form "scheduled agent", it hands off
here and uses these create steps.

Afterwards, mention `ivar agent feedback <run-id> --rating up|down` so the user
can record whether a run was useful.

## Remove an agent

Applies to any agent, a pending Agent Task included, and to the `ivar-automate`
handoff for retiring an agent.

1. Confirm with the human which agent goes.
2. Delete `agents/<name>.md`.
3. Ask before `ivar agent sync`: it removes the agent's timer from this machine.
4. Tell the human to commit the removal.
5. Run `ivar automation decisions`. If an `accepted` decision has this file's path
   (`agents/<name>.md`) as its artifact, run `ivar automation undo <id>` so the
   request can be proposed again if it keeps recurring.
