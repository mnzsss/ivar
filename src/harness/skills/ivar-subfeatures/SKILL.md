---
name: ivar-subfeatures
description: Split a feature into child features and run them in parallel — one session per child, integrate each as it finishes, deliver the parent once.
argument-hint: [parent-feature]
---

# ivar-subfeatures

Use this when one feature — in one repo or several — breaks into slices that
can be built in parallel: each slice becomes a **child** feature with its own
branch and session, and lands on the **parent's** branch, not on `main`.
Only the parent delivers.

## 1. Promote the parent and land Wave 0

A child's branch starts from its parent's branch, so the parent must promote
every repo a child will touch:

```bash
ivar feature promote <parent> <repo>
```

Promoting a child into a repo its parent lacks asks to promote the parent
(interactive) or refuses with that exact command (`--json`, CI, no TTY).

**Wave 0 on the parent first.** Anything two children would both edit lands
and is committed on the parent before any child exists: dependency manifests
(`pubspec.yaml`, `Cargo.toml`, `package.json`), route tables, theme or design
tokens, l10n `.arb` files, and shared registries or modules a child would add
one line to. Children branch from that commit and each owns disjoint files;
parallel edits to one shared file conflict at integrate.

## 2. Create the children and write their plans

```bash
ivar feature create <child> --parent <parent>
ivar feature promote <child> <repo>
```

One child per slice that can be built and reviewed on its own. Name them
semantically; no ticket ids.

Integration needs each child's plan gate approved, and a child session needs a
slice to execute. The parent writes it: scaffold with `ivar plan create <child> plan`,
write the child's `plan.md` from its slice of the parent plan — `repos:`, waves
with lightweight validation, the files it owns, design refs (Figma node URLs) —
plus its task packets, then approve that written plan:

```bash
ivar plan approve <child> plan
```

Never approve the empty scaffold.

## 3. Start one detached session per child

```bash
ivar session start <child> --detached
```

Print one launch line per child for the human, e.g.
`ivar session sandbox --session <id> -- claude`, and never launch a provider yourself.
Each child runs `/ivar-execute` (goal mode suits a well-specified slice) and
ends at `ivar feature execute finish`. Keep the parent's session for coordination.

## 4. Watch the tree

```bash
ivar feature status <parent> --recursive
```

## 5. Integrate each child as it finishes

Once the child's run is finished and the human has stopped its session
(`ivar session stop <id>`), integrate from the parent session, one child at a time:

```bash
ivar feature integrate <child>
```

Integrate leaves first: a child with its own children integrates after them.
A refusal names its fix command — run it, then integrate again.

## 6. Deliver the parent

Once every child is integrated (or abandoned):

```bash
ivar feature deliver <parent> --preview
```

Then hand over to `/ivar-deliver` to apply with the preview's fingerprint.
