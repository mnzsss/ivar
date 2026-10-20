---
schedule: "0 9 * * 1-5"
tool: claude
model: claude-sonnet-5-5
network: [api.anthropic.com, api.github.com]
secrets: [ANTHROPIC_API_KEY, GITHUB_TOKEN]
notify: https://ntfy.sh/my-hall-agents
timeout: 15m
---
List the open pull requests in every repo under /workspace using the GitHub
API and `GITHUB_TOKEN`. For each one, give its title, author, age in days and
a one-line summary of what it changes. Flag any pull request with no activity
for more than three days. Reply with the report only.
