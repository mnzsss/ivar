---
description: Review and maintain human-confirmed relationships between repositories in HALL.md.
argument-hint: [repo-name]
---

# Repository relations

`/ivar-relations` maintains the **Repository relationships** region of
`HALL.md`: human-confirmed, directed sentences from one registered Repo to
another. A relation expresses co-belonging — when work starts in the source
Repo, the destination Repo is worth considering too. It does **not** assert
dependency, permission, build order, merge order, or automatic promotion.

`ivar` itself never reads this region. Rust does not parse, validate, or
rewrite the relation prose; the workflow below is the entire contract.

## Prerequisites

- You must be inside a **hall** (`ivar status` succeeds). If you are in a
  session, the hall is reachable by walk-up from `$IVAR_SESSION_PATH`.
- Read `ivar.json` for the registered Repos and their deterministic order
  (the order they appear in `repos`).

## Entry modes

- **With a Repo argument** (`/ivar-relations api`, arriving as `$ARGUMENTS`):
  evaluate that Repo against every other registered Repo in both directions,
  and load every existing relation involving it.
- **Without an argument** (`$ARGUMENTS` is empty): enter review mode — read
  all existing relations,
  ask which Repo or ordered pair the human wants to review, then use the same
  flow.
- **Unknown Repo**: refuse conversationally before proposing anything — say
  the name is not registered and list the registered names.

## Evidence gathering

Before asking the human anything, read:

1. `ivar.json` — registered names and Repo order;
2. `HALL.md` — current relation sentences and their linked detailed topics;
3. relevant repository manifests, imports, workspace files, generated
   clients, API schemas, automation, and other concrete cross-repository
   signals.

Evidence may motivate a proposal but can **never** become a relation without
human confirmation. Mere co-occurrence in a feature is not evidence.

## Conversation order

1. Present **one proposal at a time**.
2. Name the source Repo, the destination Repo, the concrete evidence, and the
   proposed human-readable sentence.
3. Ask the human to **confirm**, **correct**, or **reject** it.
4. A correction is repeated back as the proposed final sentence.
5. After five proposals, ask whether to continue. Five is a soft limit:
   continue only when the human asks to.
6. After code-derived proposals, ask one open question for relationships the
   repositories do not reveal.
7. Show one final summary of additions, sentence replacements, removals,
   topic creations/changes, and newly orphaned topics.
8. Write **only after explicit confirmation** of that summary.

Rejected proposals are not persisted and may be rediscovered on a later run.
Stopping at the soft limit persists only what the human confirmed in the final
summary; unreviewed candidates remain unstored.

## Review and mutation rules

- Repeating an ordered pair proposes **replacing** its sentence — never merge
  two reasons or create duplicate bullets.
- Adding shows the pair and sentence.
- Correcting shows before and after.
- Removing names the pair and its current sentence.
- Every mutation requires human confirmation.
- Edit `HALL.md` directly. Never edit `CLAUDE.md`, `AGENTS.md`, or a
  session's provider-native instruction file — they are aliases or derived
  views, not sources.
- Before writing, re-read `ivar.json` and `HALL.md`. If the relation region
  changed since it was read, stop and reconcile conversationally — never
  overwrite a concurrent change.

## Canonical relation region

`/ivar-relations` owns only the bytes between its markers, always **outside**
the `ivar`-managed block:

```markdown
<!-- ivar:relations:start -->
## Repository relationships

### When working in `api`

- Also consider `web`: `web` consumes the REST contract exposed by `api`. [Context](docs/repo-relations/001-api-contract.md)
<!-- ivar:relations:end -->
```

Rules:

- Group by source Repo with `### When working in <repo>`.
- Render one bullet per ordered pair with one short human-authored sentence.
- A context link is optional.
- Order source groups and destination bullets by Repo order in `ivar.json`.
- Normalize after every confirmed mutation and remove empty source groups.
- A review with no relations writes no marker, placeholder, or status.
- Removing the final relation removes the entire region.

## Detailed relation topics

When examples, constraints, or history would make the index noisy, propose a
user-owned Markdown topic:

```text
docs/repo-relations/001-<slug>.md
```

- Create topics only on demand; never scaffold empty files.
- Use the next `max + 1` three-digit number. Never reuse or renumber
  identifiers.
- A slug may change with the title if every link is updated in the same
  confirmed mutation.
- One topic may support several related relation bullets.
- The format is a Markdown title and minimal free-form prose; there is no
  required frontmatter or managed block.
- Creating or changing a topic requires human confirmation.
- Removing a relation never deletes its topic automatically.
- When a topic loses its final link, identify it as orphaned and ask whether
  to retain it as history or remove it.

## When to use

- After `ivar repo add` tells you to run `/ivar-relations <repo>`.
- When code evidence shows two registered repos belong together.
- To review, correct, or remove existing relationship sentences.
