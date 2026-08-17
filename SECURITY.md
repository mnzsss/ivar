# Security policy

## Reporting a vulnerability

**Do not open a public issue.** Report privately, either way:

- A [private security advisory](https://github.com/mnzsss/ivar/security/advisories/new) — preferred, because the fix and the disclosure stay in one place.
- Email <menezes@mnzs.dev>, subject prefixed `ivar security`.

Include what you need to reproduce it: the `ivar` version (`ivar --version`), the
OS, the commands in order, and what happened that should not have.

**No response-time promise.** This is maintained alongside other work — the same
sentence [CONTRIBUTING.md](CONTRIBUTING.md) makes about issues, and it is not
softened here because a report is security-flavoured. Reports are read. If a
week passes with no reply, send a follow-up rather than assuming it was
received.

If you want credit in the advisory and the release notes, say so and name how
you want to be named. If you do not, that is the default.

## Supported versions

Pre-`0.1.0`, only the latest release is supported. A fix ships as a new release;
nothing is backported.

## What `ivar` is, for threat-modelling purposes

`ivar` is a local CLI. It has **no server, no account, no index and no
telemetry** — not even opt-in. That is architectural and you can check it: there
is no network client in this repo other than `ureq`, which talks to the GitHub
API and to nothing else. So there is no hosted surface to attack, and no
infrastructure disclosure to coordinate with. Everything below is on the machine
that runs the binary.

`ivar` does hold real power on that machine: it writes to your git repos, runs
subprocesses, creates symlinks, and acts as you against GitHub.

## Designed behaviour, not vulnerabilities

These are documented in [Getting started](docs/getting-started.md) and
[Concepts](docs/concepts.md). Reporting them is welcome as an issue about the
docs, not as an advisory:

- **A hall executes code it carries.** `.ivar/setups/<repo>.sh` and
  `.ivar/setups/<repo>.session.sh` are shell scripts committed to the hall, and
  `ivar sync` and `ivar session start` run them. **Cloning a stranger's hall and
  running `ivar sync` runs their shell script on your machine.** That is the
  feature — it is what makes one command produce a working checkout instead of a
  bare one — and it means a hall deserves exactly the trust you would give any
  repo whose `package.json` has a `postinstall`.
- **Skills are installed from git repos** and materialised into each harness's
  native config directory. Same trust boundary, same reasoning.
- **`ivar` acts as you against GitHub.** It stores no credential of its own; it
  uses `gh`, then `GITHUB_TOKEN`/`GH_TOKEN`, and stops if it has neither. It
  cannot exceed the permissions of the token you already have.
- **`ivar` runs the agent harness you point it at.** What that agent does inside
  a session is between you and your harness.

## What is in scope

Anything where `ivar` exceeds the authority a user knowingly gave it:

- Writing outside the hall — path traversal out of `.ivar/`, a symlink in the
  view dir resolving somewhere it should not, a repo name that escapes its
  directory.
- A credential or token reaching a log line, an error message, a `--json`
  payload, a committed file, or a subprocess argument list.
- A value from `ivar.json`, a branch name, a repo name or a GitHub API response
  reaching a shell as code rather than as an argument.
- Destroying uncommitted work in a repo without the confirmation the command
  documents.
- Anything in this repo's dependency tree with a known advisory that `ivar`
  actually reaches.
