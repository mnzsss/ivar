---
name: ivar-feedback
description: Record ivar defects, unexpected guard denials, CLI friction, or workflow proposals
---

# ivar-feedback

When you encounter an ivar bug, an unexpected guard denial that seems incorrect,
confusing CLI output, or tool friction while working in a hall, record a structured
feedback entry.

## When to record feedback

- An `ivar guard` denial blocked an edit to a file that belongs to a promoted repo or session view.
- An `ivar` command crashed, panicked, failed unexpectedly, or emitted confusing diagnostics.
- An `ivar` workflow or skill lacks necessary guidance or behaves inconsistently across providers.
- You have a concrete proposal for improving ivar CLI, guards, planning, or execution.

## How to record feedback

Run `ivar feedback add` with `--kind bug` (default) or `--kind proposal`, passing the structured body via stdin:

```sh
ivar feedback add --title "<short descriptive title>" --kind bug --file - << 'EOF'
## Symptom
Describe what failed or went wrong from the user's or agent's perspective.

## Reproduction
Steps, commands, or tool payloads that reproduce the issue. Quote all file paths relative to the hall root.

## Cause
Root cause analysis if known, including relevant source locations in ivar or provider harnesses.

## What should change
Concrete proposal for the fix, expected behaviour, or interface change.

## Suggested tests
Automated unit or integration tests that would prevent regression.
EOF
```

## Critical rules

1. **Path relativity:** Always quote paths relative to the hall root (e.g. `repos/my-repo/src/lib.rs`), never absolute paths from your container or host.
2. **Never submit:** NEVER run `ivar feedback submit` yourself. Submission sends data to GitHub and requires human confirmation. After recording with `ivar feedback add`, report the entry ID to the user and tell them to run `ivar feedback submit <id>` if they wish to publish it.
