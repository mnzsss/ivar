---
schedule: "@hourly"
image: ghcr.io/my-org/site-check:1
command: [site-check, --url, https://example.com]
network: [example.com]
timeout: 2m
---
A custom `command` does not receive this prompt; describe what the agent does
here for whoever reads the file.
