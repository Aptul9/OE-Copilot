# Remote MCP for mail, calendars and Teams (src/http.ts). Node 26 runs the TypeScript sources as they are: no build step.
# Same base image digest as OE Copilot's agente-ts (index amd64 + arm64); to update: docker buildx imagetools inspect node:26-slim
FROM node:26-slim@sha256:193fe51b64e77981119c98c2002c9e32a70e2f006fb4d25068ce0558998917f0
ADD --checksum=sha256:c9d97d4b57682f664a94a6a7c81df09ff9f674dd7c900c38024ec95db4940692 --chmod=755 https://github.com/shaharia-lab/slackcli/releases/download/v0.19.1/slackcli-linux /usr/local/bin/slackcli
ENV NODE_ENV=production BIND_HOST=0.0.0.0 PORT=8190 SLACKCLI_PATH=/usr/local/bin/slackcli SLACKCLI_LOG_LEVEL=off SLACKCLI_NO_UPDATE_NOTIFIER=1
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev --ignore-scripts --no-audit --no-fund && npm cache clean --force
COPY src ./src
USER node
EXPOSE 8190
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD ["node", "-e", "fetch('http://127.0.0.1:8190/healthz').then((r) => process.exit(r.ok ? 0 : 1), () => process.exit(1))"]
CMD ["node", "src/http.ts"]
