# Upgrading

Two files change shape over time, and they have deliberately different rules.
Knowing which is which is the whole of this page.

`ivar.json` schema v4 adds discovered token and resource endpoint fields to pre-registered
OAuth entries, allowing OMP to refresh OAuth tokens automatically via rendered `auth` blocks.
| | local state | `ivar.json` |
| --- | --- | --- |
| where | `.ivar/` | the hall root |
| in git? | gitignored | **committed** |
| when the format moves | migrates itself, silently | waits for you |

## The promise

> There will never be a hall you cannot open.

Narrower than it sounds, and worth reading precisely: every version that changes
the format ships the migration for it, and **the chain is never pruned** — so a
hall written by any past version still opens with any later one.

The one case `ivar` refuses is a hall written by a **newer** binary than yours. It
names both versions and does not touch the file. A half-understood state file is
worse than none.

## Local state: nothing to do

`.ivar/state.json`, lockfiles and per-feature state migrate on read and save the
migrated form without telling you. Nobody reviews these files, nobody shares
them, and deleting them costs a re-clone rather than work — so there is nothing
useful to say about it.

## `ivar.json`: you decide

It is committed. If upgrading `ivar` quietly rewrote it, that rewrite lands in
your next commit — and a teammate still on the older binary would then refuse
your commit as a version they do not understand. **One person's upgrade would
break someone else's checkout.**

So it is a team event, and it waits:

```sh
ivar migrate
```

It shows what would change and asks. Answer `y` and it advances the version,
then tells you to commit the result.

Three things worth knowing:

- **It asks, and it will not act if nobody is there to answer.** Piped or in CI,
  it prints what it would do and changes nothing. There is no `--yes`. A schema
  bump on a committed file should not be reachable by an unattended script.
- **Commit it promptly.** Between the migration and the commit, your hall is at a
  version your teammates' binaries may not accept.
- **Coordinate the upgrade.** Everyone should be on the new `ivar` before the
  migrated `ivar.json` lands on `main`.

## When someone else migrated first

You pull the hall and every command refuses:

```
blocked: ivar.json is at schema version 2, but this build of ivar only
         understands up to version 1
```

Upgrade `ivar`. That is the whole fix — `ivar migrate` cannot help here, and
running it will tell you so rather than pretending:

```
ivar.json is at version 2; this build understands up to 1.
warning: schema version 2, but this build understands up to 1 — upgrade ivar;
         this command cannot help
```

That run exits `1`, not `0`: the command worked, but your hall is unusable as it
stands.

## Checking without changing anything

```sh
ivar migrate --json     # reports the plan; still writes nothing unattended
ivar doctor             # everything else that might be wrong
```

`ivar migrate`'s `plan` field is one of `current`, `available`, `unreachable`, or
`too_new` — safe to poll from a script, since an unattended run cannot act.

## "Reports version 0"

```
ivar.json reports version 0, and this build has no migration to reach version 1.
```

Version 0 means **no `version` field at all**, and `ivar.json`'s first public
version is 1 — so there is no v0 to migrate from. This is almost always a file
that is not an `ivar.json`: a different tool's config, or a hand-written stub.
`ivar` refuses to relabel it as current, because adopting a foreign file as its
own is the one thing the format contract forbids outright.

See [On-disk format](../reference/on-disk-format.md) for the full contract.

## MCP transport rename is a content edit, not a migration

Upgrading `ivar` also introduces a **breaking change** to the `mcp[].type`
vocabulary. The only accepted values are now `http` and `local`; the previous
spellings (`stdio`, `sse`, `streamable-http`, and the OpenCode-native `remote`)
are rejected at parse time with a diagnostic that names the canonical
replacement.

`ivar migrate` does **not** rewrite these. Migration advances the file's schema
*version*; it does not edit the *content* of a committed field. The transport
rename is a hand-edit:

| Obsolete in `ivar.json` | Canonical |
|---|---|
| `"type": "stdio"` | `"type": "local"` |
| `"type": "sse"` | `"type": "http"` |
| `"type": "streamable-http"` | `"type": "http"` |
| `"type": "remote"` | `"type": "http"` (was never a valid Ivar value) |

After editing, re-run `ivar sync` to regenerate each provider's config file.
Claude Code receives `stdio` and OpenCode receives `local` for a `local`
entry; both receive their remote spelling for an `http` entry. The full
translation table and the editor-discoverable schema are in
[On-disk format](../reference/on-disk-format.md).
