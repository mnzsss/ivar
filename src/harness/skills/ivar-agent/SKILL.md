---
name: ivar-agent
description: Create a scheduled ivar agent when the user asks for one, and suggest one when you notice a repetitive task or something the user keeps monitoring by hand
---

# ivar-agent

A scheduled agent is one Markdown file, `agents/<name>.md` at the hall root.
Its frontmatter says when and how it runs, its body is the prompt, and systemd
fires each run. The field reference is `docs/guides/scheduled-agents.md` in the
`ivar` repo; `ivar agent --help` lists the commands.

## When to suggest an agent

Suggest one when the same work keeps coming back and nobody needs to be present
for it:

- The user asks for the same check, summary or report again ("what changed since
  yesterday", "any new issues", "is CI green").
- The user watches something by hand: CI, open PRs or issues, dependency
  updates, a deploy, a log, a price, a page that changes.
- A task is described with a rhythm: "every morning", "weekly", "whenever X
  happens", "remember to check".
- You are about to run the same command sequence a third time across turns or
  sessions.

Suggest at the end of the turn, never in the middle of other work, and once per
pattern. If the user declines or ignores it, drop it.

A suggestion is a few lines: what the agent would do, a name, a schedule, whether
it only reads or may change a repo, then a yes/no question. Create nothing
before the answer.

## When an agent is the wrong tool

- The work happens once, or needs the user's decision halfway: an unattended run
  that asks for input ends as `blocked`.
- The trigger is an event, not a time. Agents run on a cron schedule only; the
  nearest fit is a frequent schedule that checks and exits when nothing changed.
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
   time zone the user means.
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
7. **Notify and timeout**: an `http(s)` URL for the run summary if they want
   one; a `timeout` when the default 30m is wrong.

Write the prompt for a reader with no context: it names the repo, says what to
look at, what to produce and where, and what "nothing to report" looks like. It
cannot ask questions.

Then:

1. Write `agents/<name>.md`. Hall files are committed straight to the hall's
   `main`, with the `commit` skill if the user asked for a commit.
2. Ask before `ivar agent sync`: it installs systemd user timers on this machine.
3. Ask before `ivar agent run <name>`: it runs the tool, which may cost money. Use
   it as the first check, then read the result with `ivar agent logs <name>`.
4. Show `ivar agent status` so the user sees the next run.
5. If `sync` or the run reports a missing requirement, run `ivar doctor` and
   relay its fix. A file that fails to load when its timer fires leaves no
   receipt; the error is only in `journalctl --user -u <unit>`.

Afterwards, mention `ivar agent feedback <run-id> --rating up|down` so the user
can record whether a run was useful.
