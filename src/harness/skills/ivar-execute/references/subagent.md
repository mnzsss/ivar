# Sub-agent dispatch template

Fill every field before dispatching. Every path is absolute — resolve it from
`$IVAR_SESSION_PATH` and the feature directory (`$IVAR_SESSION_PATH/../..`)
first. Never send a relative path: the sub-agent does not share your working
directory and must not spend rounds discovering it.

```
## Context
- Working directory: <absolute worktree root; run every command from here>
- Repository: <repo name>
- Worktree root: <absolute path to .ivar/repos/<repo>/<branch>>
- Session: <absolute $IVAR_SESSION_PATH>

## Artifacts (read before editing)
- Task packet: <absolute path to tasks/NN-<name>.md>
- Plan: <absolute path to plan.md>
- Requirements: <absolute path to requirements.md>

## Boundaries
- Allowed files: <exactly the Files section of the task packet, absolute>
- Allowed commands: <the packet's Run commands and the wave's lightweight validation>
- Do not edit other files, run other repos' commands, commit outside Step 5, or change plan.md.

## Task
Execute the task packet step by step (Red → Green → Refactor). Report the
Step 2 failure output, the Step 4 pass output, and every file you changed.
```
