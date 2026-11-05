# On-disk format: memory pages

Part of the [on-disk format](on-disk-format.md) contract: the markdown files the hall
memory indexes, where they live and what their front matter may say. The index over
them is described in [databases](on-disk-format-databases.md#pages-and-search-schema-3).

## Roots

| Root | Holds | Committed |
| --- | --- | --- |
| `docs/memory/<kind>/` | durable kinds: `product`, `update`, `decision`, `gotcha`, `procedure`, `concept` | yes |
| `.ivar/memory/<kind>/` | the ephemeral kind: `session`, as `.ivar/memory/session/<id>.md` | no, `.ivar/*` is ignored |

A page's identity is its path relative to the hall. `ivar` does not create these
directories; the first page written does. A kind directory of the wrong root (for
instance `docs/memory/session/`) or an unknown kind is not a page kind.

## Kinds and tiers

The tier follows the kind and is not written in the file.

| Kind | Tier | Numbered |
| --- | --- | --- |
| `product` | semantic | yes |
| `decision` | semantic | no |
| `concept` | semantic | no |
| `gotcha` | procedural | no |
| `procedure` | procedural | no |
| `update` | episodic | yes |
| `session` | episodic | no |

`working` is a valid value of `--tier` but no kind maps to it yet.

## Path rule

Pages are flat: `<root>/<kind>/<slug>.md`, with a kebab-case slug (lowercase letters
and digits joined by single hyphens). A file in a subdirectory of a kind, outside a kind
directory, with another extension casing or a non-kebab slug is not indexed. The
reindex lists it in `ReindexReport.invalid` and `ivar doctor` reports it as
`memory.page_misplaced`, so a misplaced page does not vanish silently.

`product` and `update` slugs start with `NNN-` (three digits). The sequence is shared
by every `update` page and every `.cleanup.json` record beside them: the next number is
the highest one in `docs/memory/update/`, counting both, plus one.

## Front matter

Front matter is optional: a file without it is a valid page. All fields are optional.

| Field | Type | Meaning |
| --- | --- | --- |
| `title` | string | falls back to the first H1, then to the slug |
| `kind` | string | must equal the directory, or the page is `memory.page_invalid` |
| `tags` | list of strings | matched exactly by `--tag` |
| `pinned` | boolean | ranks the page higher in `memory query` |
| `relations` | list of `{type, target}` | typed links; `target` is `kind/slug` |
| `expires_at` | string | `YYYY-MM-DDTHH:MM:SSZ`, UTC only |

```yaml
---
title: Page index follows the disk
tags: [memory, index]
pinned: false
relations:
  - type: supersedes
    target: decision/old-index
expires_at: 2027-01-31T00:00:00Z
---
```

Any other key is kept and reported as `memory.page_field_unknown`, a warning. A page
past `expires_at` stays on disk and in the index, and `memory query` omits it unless
`--include-expired` is passed. Nothing deletes a durable page.

## Session pages

The consolidation writes one `session` page per provider conversation, named after the
conversation's native id (or a 16-character hash of it when the id is not a safe file
name). It writes the page once the conversation ends or has been idle 30 minutes, and
rewrites it only when the conversation has new observations. A conversation with no
observation of importance 4 or more has no page, and an existing one is removed. The
front matter carries `title` and, when set through `memory_patch_page`, `pinned` and
`expires_at`; regeneration keeps both and replaces the body.

The hourly sweep of the `ivar graph mcp` leader deletes a `session` page 30 days after
the conversation's last observation, read from the marker in
`.ivar/spool/memory/_consolidate.json` or, without one, from the file's mtime.
`pinned: true` exempts the page from the 30 days, and an explicit `expires_at` always
applies. See [limitations](limitations-memory.md#session-pages).

## Links

The body links to other pages with `[[slug]]`, `[[kind/slug]]` and ordinary markdown links to `.md` files;
fenced and inline code are skipped. `relations` entries are links of source `relation`.
A wikilink resolves by slug, and a target that does not exist is stored unresolved.
