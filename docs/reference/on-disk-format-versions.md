# On-disk format: current versions

Part of the [on-disk format](on-disk-format.md) contract: the version each file
carries today and what changed at each version.

## Current versions

| file | version |
| --- | --- |
| `ivar.json` | 5 |
| `.ivar/features/<feature>/feature.json` | 3 |
| `.ivar/features/<feature>/execution/run.json` | 2 |
| `<view dir>/state.json` | 1 |
| `.ivar/agents/runs/<agent>/<run-id>.json` | 1 |
| cleanup record | 2 |

`<view dir>/state.json` is a session's record, one per View Dir
(`.ivar/sessions/<id>/` or `.ivar/features/<feature>/sessions/<id>/`). The hall
itself has no state file of its own. The cleanup record is the file
`ivar feature cleanup <feature> --record <path>` takes. Schema 1 records,
which carry `approvals.documentation`, are still accepted. A test fails the
build when this table and the version constants in the code disagree.

`ivar.json` starts at 1. There is no version 0 to migrate from: a file with no
`version` field is not an `ivar.json`, and is rejected rather than adopted. A
migration chain may begin at its format's earliest supported version — `ivar.json`'s
chain is `[1→2, 2→3, 3→4, 4→5]` — and v0 stays unreachable.

### `ivar.json` v2

v2 adds two things, both with embedded defaults so v1 files migrate cleanly:

```json
{
  "version": 2,
  "integration": { "via": "local", "strategy": "squash" },
  "repos": [
    { "name": "api", "url": "git@github.com:acme/api.git",
      "default_branch": "main", "checks": ["cargo fmt --check", "cargo test"] }
  ]
}
```

`integration` is the hall's per-field default (CLI > feature > hall > embedded)
and `checks` are the repo's ordered verification commands. Migrating `ivar.json`
from v1 is explicit — `ivar migrate` — because the file is committed.

### `ivar.json` v3

v3 adds one optional field to an `mcp` entry: `oauth`, a pre-provisioned OAuth
client registration for a server whose host rejects a harness's own dynamic
client registration (`ivar mcp auth`'s pre-registration step; `mcp.figma.com`
today).

```json
{
  "version": 3,
  "mcp": [
    {
      "name": "figma",
      "type": "http",
      "url": "https://mcp.figma.com/mcp",
      "oauth": { "client_id": "…", "client_secret_env": "IVAR_MCP_ACME_FIGMA_SECRET" }
    }
  ]
}
```

`client_id` is not a secret. `client_secret_env` is the *name* of an
environment variable — never a value — the same reference convention `env`
already follows elsewhere in an `mcp` entry. It is optional: a public OAuth
client that was issued no secret omits `client_secret_env` entirely. The
`oauth` block is absent by default, so v2 → v3 adds nothing to a hall that
never used it; migrating is still explicit (`ivar migrate`) because the file
is committed.

The `name` stored here (`figma`, above) is canonical and unqualified. Providers
never see it directly: every provider boundary — the key in `.mcp.json` and
`opencode.json`, the argument to `claude mcp login` / `opencode mcp auth`, the
key OpenCode writes into `mcp-auth.json`, and the OAuth secret variable name —
receives `<hall>-<server>` instead, derived from the hall this manifest belongs
to (`acme-figma`, in a hall named `acme`). This is a breaking change from
previous behaviour, where the stored name was used verbatim at every provider
boundary. A hall whose entry is already hall-qualified (`figma-acme`, say)
must rename it to the canonical form by hand and re-run `ivar sync`, and
re-export any `client_secret_env` secret under the new variable name — `ivar`
will not rewrite a committed, hand-edited file by guessing at intent.

### `ivar.json` `mcp[].auth` key

`auth` is an optional field on an `http` `mcp` entry. Its only value is `"oauth"`: the
server is known to require OAuth. `ivar mcp auth` writes it after a login that left a
credential in the provider's store, so `ivar mcp status` and `ivar doctor` report the
server as `authenticated`, `expired` or `missing` instead of `not-required`.

It is separate from `oauth`. `oauth` is a pre-registered client (`client_id`); `auth` carries
no client and no secret, and exists for servers that register or authenticate themselves
(CIMD, or a provider-owned login) and so never get an `oauth` block. A server that carries
`oauth` is already treated as requiring auth.

```json
{ "name": "prisda", "type": "http", "url": "https://mcp.example.com/mcp", "auth": "oauth" }
```

It adds no `ivar.json` version, and `schema/5.json` gains the property in place, the way the
other additive `mcp` fields did. An `ivar` older than the field rejects the whole file as having
an unknown field, so a hall that records it needs every teammate on a release that knows it.
A hall written before the field existed keeps reporting such servers as `not-required` until
`ivar mcp auth` runs for them again; `status` makes no network call to find out sooner. A server
that needs no auth (a login that stored no credential) is never marked.

### MCP transport migration (v2 → v3 vocabulary change)

The MCP `type` field uses the canonical names `http` (a remote server reached
over HTTP) and `local` (a local process spawned via stdio). The previously
accepted spellings — `stdio`, `sse`, and `streamable-http` in the manifest, and
the OpenCode-native `remote` — are rejected at parse time with a diagnostic that
names the canonical replacement.

This is a **breaking change**. An existing committed `ivar.json` using any of
those spellings stops loading until you edit it by hand:

| Obsolete spelling in `ivar.json` | Canonical replacement |
|---|---|
| `"type": "stdio"` | `"type": "local"` |
| `"type": "sse"` | `"type": "http"` |
| `"type": "streamable-http"` | `"type": "http"` |

`ivar migrate` advances `ivar.json`'s *schema version* (for example v1 → v2 →
v3); it does **not** rewrite transport spellings. That rename is part of the
manifest's content, not its version chain, so it is a team event like the
version bump — one person upgrades, the team agrees, then you edit and commit.
See [Migration and provider translation](on-disk-format.md#migration-and-provider-translation)
for how each canonical value materialises at each provider boundary.

### `ivar.json` `memory` key

`memory` is an optional object with two optional fields:

- `capture`, on unless set to `false`;
- `handoff_ttl_days`, the days an open handoff or a pending message stays visible. It
  must be 1 or more, and defaults to 7.

It adds no `ivar.json` version: a hall that never sets it stays at its current version,
keeps capturing and keeps the 7-day expiry.

```json
{ "memory": { "capture": false, "handoff_ttl_days": 3 } }
```

An `ivar` older than a field rejects the whole file as having an unknown field, so
a hall that sets it needs every teammate on a release that knows it. When both fields
are back at their defaults, the key is removed.

The hall memory database has its own schema version, `memory_schema.version`, which is
8 today; see [Memory database](on-disk-format-databases.md#memory-database-ivarmemorydb).

The graph database has its own schema version, 12 today. Version 12: hall `docs/memory/` files
lose their content-index rows; see
[Graph schema v12](on-disk-format-databases.md#graph-schema-v12).

### `feature.json` v3

v3 adds the nested-subfeature fields. Only the child side of the lineage is
stored — `parent` — and children are derived by scanning:

```json
{
  "version": 3,
  "name": "checkout-tax",
  "branch": "checkout-tax",
  "parent": "checkout-v2",
  "base": "checkout-v2",
  "integration": { "via": "pr" },
  "promotions": {
    "api": {
      "worktree": "ready",
      "base": "checkout-v2",
      "integration_receipt": {
        "source_sha": "…", "target_branch": "checkout-v2", "result_sha": "…",
        "via": "pr", "strategy": "squash", "pr_url": "https://github.com/…/pull/7",
        "verification": {
          "command_fingerprint": "…",
          "child": [ { "command": "cargo test", "success": true, "exit_code": 0, "diagnostic": "" } ],
          "parent": [],
          "pr_checks": [ { "name": "ci", "bucket": "pass" } ],
          "verified_at": "2026-08-14T12:00:00Z"
        }
      }
    }
  }
}
```

`parent` and `integration` default to absent, and `integration_receipt` to null,
so v2 files migrate in place — local state migrates silently on read. No feature
stores a child list, and no lifecycle field is persisted: the integration state
is derived from the close record plus receipt freshness. Child branches and
worktrees are retained after integration so receipt validation stays exact.
