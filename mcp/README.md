# Communication MCP

One remote MCP endpoint for mail, calendars and chats: Outlook mail and calendars of every mailbox OE Copilot is signed in to (Outlook REST v2.0), Gmail and Google Chat through the OE Copilot agent, Teams through TeamsRelay `/mcp`, Slack through its Web API. Clients hold one bearer token; every backend credential stays on the server.

The server signs nothing in and stores no token on disk. OE Copilot owns sign-in and renewal of every mailbox and Google grant, TeamsRelay owns the Teams sessions, the OE autologin keeps the Slack session in the slackcli workspace store.

Rust (`src/`), since 2026-11-14. It replaces the TypeScript server, kept in `ts/` as the reference it was checked against: same tools, input schemas (`src/tools.json` is the TypeScript `tools/list`), validation messages, resource handles and answers. One static binary in a `FROM scratch` image, under 1 MiB of memory at rest (the TypeScript server used 125 to 220 MiB).

```text
MCP client --HTTPS, Bearer MCP_HTTP_TOKEN--> TeamsRelay Caddy --> communication-mcp:8190/mcp
  communication-mcp --> agent:8180 (OE Copilot: token broker, Gmail and Google Chat API) --> Outlook REST, Google
  communication-mcp --> webapp:8090/mcp (TeamsRelay, its MCP_TOKEN) --> Teams
  communication-mcp --> https://<workspace>.slack.com/api (session from /slackcli/workspaces.json) --> Slack
```

## Remote server

Streamable HTTP, stateless, as the TypeScript SDK 2.4 served 2025-era clients: every POST is one exchange, requests are answered on an SSE stream (one event per answer, a `: keepalive` comment every 15 s), notifications and responses get 202.

| Path | Answer |
|---|---|
| `POST /mcp` | MCP. Needs `Authorization: Bearer <MCP_HTTP_TOKEN>` (constant-time compare), a `Host` listed in `MCP_HTTP_HOSTS` (421 otherwise: DNS rebinding), no `Origin` header (403: no browser callers), body up to 4 MiB (413), `Content-Type: application/json` (415), `Accept` with both `application/json` and `text/event-stream` (406) |
| `GET`, `DELETE /mcp` | 405 `Method not allowed.`: no sessions |
| `GET /healthz` | `{"ok":true}`, no auth, no backend call |
| `/everflow/*` | Read-only routes for Everflow, `Authorization: Bearer <EVERFLOW_TOKEN>`; see "Everflow" |

| Variable | Default | Meaning |
|---|---|---|
| `MCP_HTTP_TOKEN` | | Client bearer token, at least 32 characters |
| `EVERFLOW_TOKEN` | | Bearer of `/everflow/*`, at least 32 characters, different from `MCP_HTTP_TOKEN`; unset = those routes answer 404 |
| `MCP_HTTP_HOSTS` | | Comma-separated `Host` values accepted on `/mcp` |
| `OUTLOOK_MCP_BROKER_URL` | | OE Copilot token broker (`http://agent:8180/api/token-posta`); Gmail and Google Chat go to the same origin |
| `OUTLOOK_MCP_BROKER_KEY` | | Agent `API_KEY` |
| `OUTLOOK_MCP_TEAMS_URL`, `OUTLOOK_MCP_TEAMS_TOKEN` | | TeamsRelay `/mcp` and its `MCP_TOKEN`; empty URL = no Teams |
| `SLACK_WORKSPACES` | `/slackcli/workspaces.json` (image) | slackcli workspace store; unset = no Slack |
| `COMMUNICATION_ID_KEY` | | Key of the resource handles; compose sets it to `MCP_HTTP_TOKEN`, so handles survive restarts |
| `PORT` | `8190` | Listen port |
| `BIND_HOST` | `127.0.0.1` (`0.0.0.0` in the image) | Listen address |

Backend addresses that receive a credential must be https, or plain http to loopback or to a single-label container name. No redirect is followed with a credential. The Outlook API address is fixed in the code: a broker answer or a pagination link cannot send a token anywhere else.

`communication-mcp --health` asks the running server for `/healthz` (the container healthcheck: the image has no shell and no curl).

### Deployment

`compose.yml`, on the host of OE Copilot and TeamsRelay (`oe-vm`, `/opt/communication-mcp`). The container joins `oe-copilot_default` and `teamsrelay_default`, reaches `agent:8180` and `webapp:8090` there, mounts `/opt/communication-mcp-state/slackcli` read-only at `/slackcli`, and listens on `127.0.0.1:8190` only. A push on `main` touching `mcp/**` deploys it (`.github/workflows/mcp.yml`: rsync, `.env` from the `mcp` environment, `docker compose up -d --build mcp` in the one compose project of oe-vm (`deploy/stack/README.md`), health check, rollback to the previous tree on failure). The first build compiles the dependencies (a few minutes); BuildKit cache mounts keep later builds incremental.

Public address: a path on the TeamsRelay name, `/communication/*`, which the TeamsRelay Caddy strips; `MCP_HTTP_HOSTS` lists that name.

Client registration, OpenCode (`opencode.jsonc`):

```jsonc
"mcp": {
  "communication": {
    "type": "remote",
    "url": "https://teams.example.com/communication/mcp",
    "headers": { "Authorization": "Bearer {env:COMMUNICATION_MCP_TOKEN}" }
  }
}
```

Claude Code: `claude mcp add --transport http --scope user communication https://teams.example.com/communication/mcp --header "Authorization: Bearer <token>"`.

### Everflow

Everflow reads every source it takes from this host through three routes, with one read-only token (`src/everflow.rs`). `EVERFLOW_TOKEN` opens only these routes and `MCP_HTTP_TOKEN` never does; `Host` and `Origin` rules are those of `/mcp`. The routes forward the requests Everflow already speaks, so its parsers see the same bodies as before.

| Route | Request | Answer |
|---|---|---|
| `POST /everflow/agent` | `{"requests": [{"path", "body"?}]}`, at most 100. Paths: `/api/communication/accounts` (no body), `/api/communication/read`, `/api/communication/gmail/read`, `/api/communication/gchat/read` (object body). Anything else: 400 before any call | `{"responses": [{"status", "body"}]}` in request order: the agent's status and body as text; agent unreachable = 502 for that request |
| `POST /everflow/slack` | `{"workspace": "T...", "requests": [{"method", "params"?}]}`, at most 100, string params. Methods: `auth.test`, `client.counts`, `subscriptions.thread.getView`, `conversations.history`, `conversations.replies`, `conversations.info`, `users.info`. Unknown workspace: 404 | The same shape, Slack's status and body; the session comes from the slackcli store, through the Slack limiter, no retry |
| `GET /everflow/teams?since=<ms>&max_rows=<n>` | `max_rows` 1 to 100000 | TeamsRelay `GET /api/everflow/saved` with its `MCP_TOKEN`, body unchanged on 200; any other status or no answer = 502 `{"error"}` |

An answer above 64 MiB stops the call with 507 `{"error":"answer too large"}` (Everflow: `<source>.result-limit`), never a cut body. A backend that is off (no Teams, no Slack, no agent) answers 404 on its route.

## Unified contract

Tools: `communication_accounts`, `email_folders|list|search|read|thread|send|reply|reply_all|forward|draft_update|draft_send|draft_delete|manage|attachments|attachment`, `chat_list|search|read|send|edit|delete|react|attachment`, `calendar_list`, `calendar_events`. Every data operation requires an exact `account_id` from the inventory. Resource ids are signed handles bound to one account and one kind; native ids cannot be substituted. Unsupported operations return `UNSUPPORTED_CAPABILITY`. No write is retried after an ambiguous transport failure.

Mail compose, same schema for Outlook and Gmail: `email_send` (to, cc, bcc, reply_to, files, draft; HTML body when the text looks like HTML), `email_reply` and `email_reply_all` (thread kept, original quoted, to/cc added to the original recipients), `email_forward` (comment, original attachments kept), `email_draft_update` (recipient lists replace, text replaces the author part and keeps the quote, files added), `email_draft_send`, `email_draft_delete`. `email_send.message_id` is the old reply path (recipients replace the original ones, no quote).

Outlook lists follow same-collection pagination links (at most 20 pages) and report `has_more`, `incomplete` and `stop_reason`. Teams reads its saved chat window: no search, no threads, no uploads. Google Chat search scans recent topics.

Slack: the calls slackcli 0.19.1 made, from the same workspace store (browser session: `xoxc` token and `d` cookie, sent only to the https `*.slack.com` workspace origin), with its pacing (two calls at a time, 200 ms between starts) and retries (a 429 for any call, a 5xx or network failure for reads only). Answers keep the slackcli `--json` shapes. Edit and delete check that the message is the user's own.

## Differences from the TypeScript server

- No local mode: no stdio entry, no SSH or service-gateway token source, no local file attachments. `ts/` still has them.
- Slack without the slackcli subprocess: a failed Slack call reads `Slack request failed; write outcome may be unknown`; an uploaded file keeps its name (slackcli saw it as `0-<name>`).
- `serverInfo.version` is `5.0.0`.
- The 2026-07-28 protocol revision (per-request `_meta` envelope) is not served; no client uses it.

## Checks

`scripts/check.sh mcp` runs `cargo test --locked` (unit tests, and `tests/server.rs`: the whole server over HTTP against fake broker, Outlook, agent, TeamsRelay and Slack) and the TypeScript suite in `ts/`; `--docker` builds the image. Cargo in a Linux Rust image or WSL: the Windows GNU toolchain has no C compiler for ring.
