# Scheduled agents: blocked runs

Part of [Scheduled agents](scheduled-agents.md).

## The provider check

Before a `runtime: host` run launches anything, `ivar` checks the agent's `tool`:

- **installed and answering:** the tool is found on the run's `PATH` and `<tool>
  --version` exits 0 within 10 seconds;
- **logged in:** the tool's login file holds a login (a non-empty JSON object), or
  `secrets` lists one of the tool's API key variables (`ANTHROPIC_API_KEY`,
  `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` for `claude`; `OPENAI_API_KEY`,
  `CODEX_API_KEY` for `codex`; `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
  `OPENROUTER_API_KEY`, `GEMINI_API_KEY` for `opencode`). `ivar` reads the file; it does
  not ask the provider whether the token is still honoured, so an expired or revoked
  login shows in the run itself.

The check runs for every run, whether the timer fires it, you run `ivar agent run`, or
`ivar agent resume` runs it again, after the script check and before the DNS wait. When
it fails, the tool is not launched and the receipt is `blocked`, with the reason as its
output:

- `agent.provider_unavailable: provider unavailable: ...` when the tool is missing or
  does not run;
- `agent.provider_auth: auth error: ...` when no login is found, naming the file
  looked for.

Fix the cause, then run `ivar agent resume <name>`. A `runtime: docker` run is not
checked: its tool lives in the image and its login is whatever `secrets` carries, which
the host cannot see. Neither is an agent with `image:`.

## Resuming blocked runs

`ivar agent resume` runs again the agents whose latest run is `blocked`; without a name
it takes every such agent in the hall. A run is blocked when the tool asked for
something (a pending approval, or `BLOCKED:` declared by the run, see [Outcomes](scheduled-agents.md#outcomes)),
when the provider check failed, or when `ivar` refused a precondition before launching
(for example `agent.secrets_mode`, `agent.landlock_unavailable`). Receipts an older
`ivar` recorded as `failed` with a reason starting `blocked:` count too.

Each resumed agent runs through the same path as `ivar agent run --wait`: the provider
check, the agent's lock, a receipt with `trigger: manual`, the agent's `notify` and the
hall's `problem_notify`. The agents run one after the other, and the command waits for
all of them:

    $ ivar agent resume
    nightly resumed 20261101T070000Z-1a2b3c4d: succeeded (20261102T091500Z-5e6f7a8b)
    triage resumed 20261101T073000Z-9c0d1e2f: blocked (20261102T091730Z-3a4b5c6d)

With `--json`, `resumed` lists `agent`, `blocked_run`, `run_id` and `outcome` for each.
Nothing blocked prints `No blocked agent runs to resume.` and exits 0. `ivar agent
resume <name>` for an agent whose latest run is not blocked fails with
`agent.not_blocked` and names that run's outcome. An agent file that no longer
validates is skipped with an `agent.invalid` warning and the others still run. A run
that is blocked again stays resumable. The command never retries by itself: nothing
runs again until you ask.

A concluded Agent Task has left `agents/`, so it is not resumed; copy it back from
`agents/done/` with a new `at`.

## Hall problem notifier

`ivar.json` can name one place to tell about problems, beyond each agent's own `notify`:

    {
      "agents": { "problem_notify": "telegram:-1001234567890" }
    }

`problem_notify` takes what `notify` takes: an `http(s)://` URL that receives a JSON
summary, or `telegram:<chat_id>`, which uses `TELEGRAM_BOT_TOKEN` from
`.ivar/secrets/agents.env` as described under [Telegram](scheduled-agents.md#telegram). A value that is
neither is refused when `ivar.json` is read (`manifest.invalid_problem_notify`). It is
optional; without it nothing changes.

It is told when a run ends `blocked`, whatever the cause: a failed provider check, a
tool waiting for approval, a declared `BLOCKED:`, a refused precondition. The message is
the run's summary, with the reason in its tail. The webhook body is the same JSON as
`notify`'s plus `"kind": "agent_problem"`, so a receiver can tell the two apart. When the
agent's own `notify` is the same target, it is not sent twice.

A failed delivery is recorded in the receipt's `notify_error`, prefixed
`problem notifier:`, with any error from the agent's own `notify` kept before it, and
never changes the run's outcome. Changing `problem_notify` needs no `ivar agent sync`.
