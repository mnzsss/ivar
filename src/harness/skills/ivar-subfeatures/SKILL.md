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

## 1. Promote the parent first

A child's branch starts from its parent's branch, so the parent must promote
every repo a child will touch:

```bash
ivar feature promote <parent> <repo>
```

Promoting a child into a repo its parent lacks asks to promote the parent
(interactive) or refuses with that exact command (`--json`, CI, no TTY).

## 2. Create the children

```bash
ivar feature create <child> --parent <parent>
ivar feature promote <child> <repo>
```

One child per slice that can be built and reviewed on its own. Name them
semantically; no ticket ids.

## 3. Run one session per child

Each child gets its own session — its own terminal or agent:

```bash
ivar session start <child>
```

Children run in parallel; nothing here starts them for you. Keep the parent's
own session for coordination.

## 4. Watch the tree

```bash
ivar feature status <parent> --recursive
```

## 5. Integrate each child as it finishes

Integration needs the child's plan gate approved. A child planned with
`/ivar-plan` already has it; for a small slice, the plan alone is enough:

```bash
ivar plan create <child> plan
ivar plan approve <child> plan
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
