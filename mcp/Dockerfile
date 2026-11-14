# Remote MCP for mail, calendars and chats (src/http.rs), in Rust. The image holds only the static binary: reqwest carries the certificate
# roots, Slack goes over its Web API (no slackcli), so no system files, no shell, no node. Healthcheck: `/communication-mcp --health`.
# Same base as the OE Copilot agent, pinned by digest (amd64 + arm64 index); to update: docker buildx imagetools inspect rust:1-alpine
FROM rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# dependencies and build stay in the BuildKit cache between deploys; the binary leaves the cache in the same step
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin communication-mcp && cp target/release/communication-mcp /communication-mcp

FROM scratch
COPY --from=build /communication-mcp /communication-mcp
ENV BIND_HOST=0.0.0.0 PORT=8190 SLACK_WORKSPACES=/slackcli/workspaces.json
# the slackcli store belongs to uid 1000 (mode 0600 in a 0700 directory), mounted read-only
USER 1000:1000
EXPOSE 8190
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["/communication-mcp", "--health"]
ENTRYPOINT ["/communication-mcp"]
