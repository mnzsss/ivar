# Example agents

Copy any of these into your hall's `agents/` directory, adjust it, then run
`ivar agent sync`. The file name is the agent's name. Every field is
described in [the scheduled agents guide](../../docs/guides/scheduled-agents.md).

| File | Shows |
|---|---|
| `pr-digest.md` | A read-only report on weekdays, with a `notify` webhook |
| `dependency-audit.md` | A model gateway on the host (`host.docker.internal`) |
| `lint-fix.md` | `access: feature`: changes land in a feature for you to deliver |
| `custom-image.md` | Your own image and command instead of a `tool` |

Secrets named in `secrets` are read from `.ivar/secrets/agents.env`, which
must be mode `0600`:

```sh
install -m 600 /dev/null .ivar/secrets/agents.env
echo 'ANTHROPIC_API_KEY=...' >> .ivar/secrets/agents.env
```

`dependency-audit.md` reaches a gateway such as OmniRoute running on the host.
Put its URL in `agents.env`, for example
`OPENAI_BASE_URL=http://host.docker.internal:20128/v1`. The egress proxy
forwards plain HTTP on any port to an allowed host, but HTTPS only on 443 and 80.
