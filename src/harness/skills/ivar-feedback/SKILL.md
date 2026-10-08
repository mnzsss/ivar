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
2. **Publish only after a human approves the exact text:** Never publish on your own initiative. When the user asks you to publish an entry:
   1. Run `ivar feedback submit <id> --preview` (add `--repo <owner/name>` if the user named another repository). It prints the redacted title and body exactly as they will be published, the target repo, and a fingerprint. It publishes nothing.
   2. Show the user that preview verbatim and ask them to approve it.
   3. Only after they explicitly approve in the conversation, run `ivar feedback submit <id> --repo <repo> --fingerprint <fingerprint>` with the fingerprint from that preview.
   4. If it fails with `feedback.fingerprint_mismatch`, the entry, its redaction or the repo changed: preview again and ask again. Never reuse an old approval.
