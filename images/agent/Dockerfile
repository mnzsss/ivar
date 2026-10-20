FROM node:22-bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends git ca-certificates curl ripgrep \
 && rm -rf /var/lib/apt/lists/*
RUN npm install -g @openai/codex @anthropic-ai/claude-code opencode-ai \
 && npm cache clean --force
RUN userdel -r node && useradd --uid 1000 --create-home --home-dir /home/agent agent
USER 1000:1000
ENV HOME=/home/agent
WORKDIR /workspace
