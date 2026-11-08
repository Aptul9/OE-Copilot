# Google communication implementation plan

## Steps
- [x] Port Google Chat RPC and session ownership to TypeScript.
- [x] Add Gmail REST reads and confirmed writes with separate OAuth storage.
- [x] Integrate authenticated agent endpoints and portable session import.
- [x] Run regression, live read/write checks, document MCP contract and persist results.

## Approved scope

Gmail and Google Chat, reads and writes. Google Chat session imported from PC and renewed through HTTP on server. Live tests at end. Drive, Jira, document editing, UI redesign and MCP implementation excluded. Existing CLI tools remain untouched. Worktree `OE-Copilot-google`, branch `feat/google-chat-gmail`, base PR136 `f9b5da8`. No merge or deployment.

## Implementation

Use `protobufjs` against retained `proto/googlechat.proto`. Remove unfinished custom encoder and schema generator. `src/gchat.ts` ports only binary RPCs used by gweb: list/read/search, members/self, send/reply/edit/delete/react/read-state and file upload/download. No streaming channel or pblite dependency.

`src/google-web.ts` owns validated cookies, domain/path scoping, bounded redirects, rotation, atomic private storage and renewal single-flight. Verify Google identity before import, then Chat self id. Persist rotated cookies, even after rejected authentication. Never replay ambiguous writes. Response failures expose status only.

`src/gmail.ts` owns separate mail-only OAuth PKCE/import/refresh in `gmail.json`, preserving existing calendar tokens and sources. Verify imported identity through Gmail profile. Reads use explicit relative-path allowlist and bounded response size. Writes use named actions with `confirm: true`: send, draft, draft removal, label modification, trash and untrash. No permanent message deletion or settings access. Gmail response includes native pagination tokens and MIME payloads; clients render bodies.

Agent endpoints under `/api/communication/gchat/*` and `/api/communication/gmail/*` retain existing API-key authentication. `/api/auth/inizio` and `/api/auth/completa` accept new providers without changing Microsoft or calendar behavior. Session import tool accepts files, supports gweb's DPAPI session export on Windows and raw gws auth export. No Go/Rust runtime process in agent.

## Test sequence

Each component starts with failing Vitest tests. HTTP fixtures verify method/body/headers, malformed requests, expiry, account mismatch, cookie isolation, redirects, response limits, concurrent renewal, no write without confirmation and no retries after ambiguous send. Run focused tests, then `npm run typecheck` and full `npm test`. Verify Docker runtime dependency installation.

Live tests use scratch agent/state only. Read Gmail profile, two pages, one MIME body and thread. Read Chat inventory, one conversation and attachment where available. Write checks limited to own Gmail draft and existing self-only Chat test space, after checking membership. Shared mail/chat sends remain prohibited without exact-text approval. Remote-IP tests use separate scratch process on `oe-vm` loopback, never production service/state. Record outcomes, explicit gaps and owned process cleanup.

## Rollback

Source changes remain on feature branch. Existing checkout, CLI installations and live services remain unchanged. Stop scratch processes by recorded identity. Remove scratch credentials after live checks; retain nonsecret reports until owner accepts output.
