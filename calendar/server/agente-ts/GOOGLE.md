# Google communication

Gmail and Google Chat run inside the TypeScript agent. No gweb/gws subprocess runs on the server. Calendar access and existing Outlook endpoints remain unchanged. New mail and chat UI screens and MCP tools are outside this change.

## Credentials

`GOOGLE_STATE` defaults to the directory containing `TOKENS` (`/stato`). Gmail OAuth grants live in `gmail.json`. Chat browser sessions live in `google-web.json`. Files are written atomically with mode `0600`; the directory must be writable by agent uid `10001`. These files must not be committed. Tokens and cookies do not appear in account or communication responses.

Gmail login uses the existing Google OAuth client with `gmail.modify`, `openid` and `email`. Its grants are separate from calendar grants. Existing gws credentials can be imported once. Gmail identity is checked through `users.getProfile` before storing a grant.

Chat uses the private web protobuf API. Import requires an explicit email address and verifies the authenticated Chat user and email before replacing a stored session. Rotated cookies are saved. Rejected sign-in attempts invoke `RotateCookies`, followed by the actual authentication request. A successful rotation alone is not accepted as authentication. Expired sessions require a new PC import. Browser fallback never runs on the server. Session duration beyond the live tests has not been established.

## Bootstrap from the PC

The agent URL must use HTTPS or loopback HTTP, typically an SSH tunnel. Set `API_KEY` in the local process, or use `--key-file`. Secrets are never command arguments. Run these commands from `server/agente-ts`.

Import existing gws credentials through stdin:

```powershell
gws auth export --unmasked | node scripts/import-google.ts --url http://127.0.0.1:13180 --provider gmail --account user@example.com --file -
```

Import an existing Windows DPAPI-protected gweb session, without running Go:

```powershell
node scripts/import-google.ts --url http://127.0.0.1:13180 --provider gchat --account user@example.com --gweb-session
```

Capture a fresh session directly from a signed-in PC profile through the existing Playwriter extension:

```powershell
node scripts/import-google.ts --url http://127.0.0.1:13180 --provider gchat --account user@example.com --browser install:Chrome:PROFILE_KEY
```

This TypeScript helper opens its own tab, allows up to five minutes for sign-in, captures only Google cookies, closes its tab/session and removes the temporary cookie file. The selected profile and extension must already be available. No headless/cloud browser or package is installed. `--file FILE` also accepts a raw `user_agent`/`cookies` JSON export for Chat.

## HTTP contract

Every endpoint requires the agent's `Authorization: Bearer <API_KEY>`. POST bodies are JSON objects, not arrays. Native pagination tokens, MIME payloads and attachment data remain available for clients to interpret. Errors use `{"errore":"..."}` with HTTP `400`, `401`, `403`, `404`, `409`, `429` or `502`. A network timeout after a write can have an unknown outcome; never blindly replay it.

`GET /api/communication/google/accounts` returns `{"accounts":[{"account":"user@example.com","provider":"gmail|gchat","status":"ok|errore","message":"..."}]}`. Chat entries also include `self_id`.

Gmail authorization:

```json
{"provider":"gmail","email":"user@example.com"}
```

POST to `/api/auth/inizio` returns the PKCE URL and state. Complete via `/api/auth/completa` with `provider`, `codice` (callback URL or code) and `state`. Alternatively send `{"provider":"gmail","account":"user@example.com","credentials":{"client_id":"...","client_secret":"...","refresh_token":"..."}}` to import an existing grant.

Chat import sends `{"provider":"gchat","account":"user@example.com","sessione":{"user_agent":"...","cookies":[...]}}` to `/api/auth/completa`. `/api/auth/rimuovi` accepts `{"provider":"gmail|gchat","account":"user@example.com"}` and removes only that communication grant. Calendar accounts are unaffected.

### Gmail

`POST /api/communication/gmail/read`:

```json
{"account":"user@example.com","path":"messages?maxResults=100&q=is%3Aunread"}
```

Allowed paths: `profile`, `history`, `labels`, `labels/ID`, `messages`, `messages/ID`, `messages/ID/attachments/ID`, `threads`, `threads/ID`, `drafts`, `drafts/ID`. Queries include `pageToken`, `maxResults` (1..500), `q`, `labelIds`, `format` (`full`, `metadata`, `minimal`, `raw`), `metadataHeaders`, `fields`, `includeSpamTrash` and history filters. Responses retain Google's `nextPageToken`. A list page contains ids and thread ids; request each message for headers/body.

`POST /api/communication/gmail/write` requires `account`, `action` and `confirm: true`:

- `send` or `draft`: `raw` (base64url RFC2822/MIME), optional `thread_id`. Replies require matching MIME `Subject`, `References` and `In-Reply-To` headers.
- `modify`: `message_id`, `add_labels` and/or `remove_labels`. Use labels such as `UNREAD`, `STARRED` or `INBOX`.
- `trash` or `untrash`: `message_id`.
- `delete_draft`: `draft_id`. Permanently removes a draft. Permanent message deletion and account settings are not exposed.

### Google Chat

`POST /api/communication/gchat/read` requires `account` and `action`:

- `list`: returns conversations (`id`, `kind`, `name`, `unread`, `last`), `self_id`, coverage and section pagination metadata.
- `read`: `conversation` (exact id), `kind` (`space` or `dm`), optional `topics` (1..1000, default 100).
- `search`: `query`, optional `topics`. Scans recent topics in returned conversations. It is not a server-side full-history search.
- `group`: `conversation`, `kind`. Returns membership data.
- `attachment`: `token` and `mime_type` from a message attachment. Returns `data` (base64) and `mime_type`.

Read responses include message id, thread id, sender id/name, UTC time, text, edit/delete flags, attachments, Drive/link annotations and reactions. `coverage: "recent-topics"` and explicit reply limits accompany message reads. Reading does not mark a conversation read.

`POST /api/communication/gchat/write` requires `account`, `conversation`, `kind`, `action` and `confirm: true`:

- `send`: `text`, optional `thread` and `files` (`name`, `mime_type`, `data` in base64).
- `edit`: `message_id`, `text`, optional `thread` and `topics`.
- `delete`: `message_id`, optional `thread` and `topics`. Only own messages are accepted.
- `react`: `message_id`, Unicode `emoji`, optional `thread`, `remove: true` and `topics`.
- `mark_read`: explicitly marks the conversation read up to now.

Edit/delete/reaction lookup is bounded by recent topics. Edit and delete verify message ownership. Pinned identity is checked again after XSRF/session refresh before RPCs and uploads. Attachments are limited to 5 MiB decoded; RPC/Gmail responses to 8 MiB; Gmail OAuth replies to 64 KiB. Redirect destinations are restricted and content-host hops receive no Google cookies. Sign-in redirects are rejected rather than returned as attachments. Creates are never retried on transport errors or 5xx. A rejected HTTP401 may refresh authentication once.

## Traps

- Google Chat is an unofficial protocol. The checked-in `.proto` is the source schema, not a generated TypeScript table. `protobufjs` reads it directly.
- Chat topic reads have no history cursor in the requests used here. Read/unread replies are capped at 20/100 per topic. Inventory is also bounded; section metadata is returned rather than claiming complete history.
- One immediate Chat edit/reaction readback did not match during live verification. Subsequent identical runs passed. Live checks use bounded read-only readbacks; writes are never replayed to force observation.
- `confirm: true` prevents accidental writes by ordinary readers. It is not a human approval mechanism. MCP/UI callers must enforce their own approval rules.
- Gmail send/trash/untrash were checked through HTTP fixtures. Live Gmail writes used an own draft and labels only. Live Chat writes used a self-only test space.

## Checks

```powershell
npm run typecheck
npm test
npm audit --omit=dev
```

Live checks ran against a scratch agent on Windows and the Linux Docker image from `oe-vm`, including real authentication renewal and reads after container restart. No production rollout is included.
