---
schedule: "0 2 * * *"
access: feature
workdir: ivar
tool: opencode
model: anthropic/claude-sonnet-5-5
network: [api.anthropic.com]
secrets: [ANTHROPIC_API_KEY]
timeout: 45m
---
Run the repository's linter and fix every warning it reports without changing
behaviour. Keep each change minimal. If a warning cannot be fixed safely, leave
it and list it at the end of your reply.
