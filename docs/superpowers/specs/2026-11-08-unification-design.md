# OE Copilot and TeamsRelay in one repository, phase 1

## Goal

One codebase for OE Copilot (calendar), TeamsRelay and the communication MCP, in the private repository `Aptul9/OE-Copilot`. Addresses, server folders, compose projects and runtime data stay as they are. Phase 1 moves code and pipelines only; sharing code between modules is phase 2.

## Decisions (owner, 2026-11-08)

| Topic | Decision |
|---|---|
| Shape | One repository, modules side by side, each with its own image, compose project and deploy |
| Home | `Aptul9/OE-Copilot` (private), on top of its existing `main` |
| Modules | `calendar/`, `teams-relay/`, `mcp/` |
| Git history | Not imported from TeamsRelay or Outlook-MCP |
| MCP | Outlook-MCP `ts/` moves to `mcp/`; the Python stdio server stays in Outlook-MCP |
| Website login | Unchanged: each module keeps its email and password from its own secrets |
| CI | None. Checks run locally with `scripts/check.sh` before a push; workflows only deploy |
| First deploy | Same code that runs now, so it proves the pipelines without changing what runs |

## Layout

```
calendar/       danfortu/OE-Copilot 9b493d0 (live): server/, app/, README.md
teams-relay/    Aptul9/teamsrelay a26d1bb (live): app/, mobile/, caddy/, deploy/, docker-compose.yml
mcp/            Aptul9/Outlook-MCP 016e750 ts/ (deployed from a CRLF working tree of the same commit)
scripts/check.sh
.github/workflows/  calendar.yml, teams-relay.yml, teams-relay-android.yml, mcp.yml
```

Paths inside each module are unchanged, so the trees that reach the server are the same and the compose project names (`oe-copilot`, `teamsrelay`, `communication-mcp`) and networks (`oe-copilot_default`, `teamsrelay_default`) stay.

## Deploy

| Module | Trigger | Target | User |
|---|---|---|---|
| calendar | push to `main` on `calendar/server/**`, dispatch | `/opt/oe-copilot` | `oecopilot` |
| teams-relay | push to `main` on `teams-relay/**` except `mobile/`, dispatch | `/opt/teamsrelay` | `teamsrelay` |
| mcp | push to `main` on `mcp/**`, dispatch | `/opt/communication-mcp` | `teamsrelay` |
| teams-relay Android | dispatch only, debug APK as artifact | none | none |

The calendar and teams-relay deploy jobs are the old ones with the module folder added. The MCP job is new: backup tgz, rsync, `.env` from the environment, `docker compose up -d --build`, `/healthz` on `127.0.0.1:8190`, rollback from the tgz when it does not answer. `/opt/communication-mcp` was hand-copied and root-owned; it moves to `teamsrelay`.

## Secrets

- Environments `calendar`, `teams-relay`, `mcp`, each with its own `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`, `DEPLOY_KNOWN_HOSTS`.
- Repository secrets only for values two modules share: `API_KEY` (calendar agent, MCP broker key) and `MCP_TOKEN` (TeamsRelay `/mcp`, MCP Teams token). The old OE repository secrets of `Aptul9/OE-Copilot` move to `calendar`: left at repository level, `BETTER_AUTH_SECRET` would reach the TeamsRelay `.env`, which has none today.
- Values copied from the running containers and `.env` files on oe-vm, and from the lockbox for the Android keystore and `google-services.json`. Before the first deploy a temporary dispatch job compares, on the server, the SHA-256 of each secret with the live value, and the rendered TeamsRelay `.env` with the one on disk.
- New deploy key per module. The keys of `danfortu/OE-Copilot` and `Aptul9/teamsrelay` are removed from the server once the new ones have deployed, so the old repositories cannot deploy.

## Cutover

1. Workflow `Server` of `Aptul9/OE-Copilot` disabled before any push.
2. Branch `feat/unified-repo`: imported trees, workflows, `scripts/check.sh`, this spec. All module checks pass locally.
3. Environments, secrets, variables and deploy keys; comparison job green.
4. Merge with `[skip ci]`, then dispatch mcp, calendar, teams-relay, one at a time.
5. Old deploy keys removed; `Aptul9/teamsrelay` archived.

## Proof per deploy

- `.versione`, `.deployed-sha` hold the new commit.
- calendar and teams-relay: `StartedAt` and `com.docker.compose.config-hash` of every container unchanged (no restart). A restart of `teams-browsers` can sign out the Npo account.
- mcp: the image rebuilds once (LF checkout instead of the CRLF copy), so the container restarts once; `tools/list` returns 24 tools.
- `https://calendar.everbright.it/login` 200, `https://teams.everbright.it/healthz` 200, TeamsRelay accounts green.

## Phase 2 outcome (2026-11-08)

Done:
- Calendar Gmail and Chat backend (PR 7) deployed: only the agent restarted.
- Retired phone stack removed from `calendar/` (194 files): `server/schermo-web`, `server/supervisore`, `server/immagine-telefono`, `server/apk`, the phone scripts of `server/strumenti`, their README sections reduced to one line with the numbers kept.
- TeamsRelay status-green fix (PR 9) and docs with the MCP-only accounts spec (PR 10) in one deploy, one restart of `teams-browsers`.

Decided against, for now:
- **npm workspaces and shared `packages/`.** The duplication is small and already diverges: the 14 shadcn components both apps carry differ per app (`cn` import, `scroll-thin`); the two auth configs share about 15 lines of options and have different plugin sets; the Microsoft web-session code runs in two runtimes (calendar agent: plain Node, no bundler; TeamsRelay light agent: esbuild bundle) with different clients and redirects; web push is an own RFC 8291 implementation in calendar and the `web-push` library in TeamsRelay. Sharing them means widening the build contexts of `calendar/server/web-ts` and `teams-relay/app` to the repository root, changing the deploy rsync scopes and compose contexts, and one hoisted lockfile for both apps: the first such deploy restarts every container, `teams-browsers` included. Worth it once a change has to be made twice.
- **Removing auth plugins.** Calendar `twoFactor` and `passkey` are the `AUTH_2FA` switch (off today, not dead). TeamsRelay `admin` multi-user and MCP OAuth are features of TeamsRelay with no second user or client today. Removing them is a scope decision, not cleanup.

Held by the owner: PR 8, `/communication` route of the MCP (go-live).

## Phase 2 (as planned before)

- npm workspaces and `packages/`: the 14 shadcn components both apps carry, the email and password auth core, the Microsoft web session module (`agente-ts/src/sessione-web.ts`, `app/src/agent/light/session-auth.ts`), web push.
- Remove the retired phone stack of calendar (`server/schermo-web`, `supervisore`, `immagine-telefono`, `apk`, `strumenti` leftovers).
- Unused auth plugins: calendar `twoFactor` and `passkey` (off), TeamsRelay `admin` multi-user and MCP OAuth (no client).
- Pending module work: calendar Gmail and Chat backend (`danfortu/OE-Copilot` `20b0d37`, never deployed), TeamsRelay docs `62c1adb`, MCP-only accounts spec, `/communication` route (TeamsRelay PR 143).
