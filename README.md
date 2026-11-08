# OE Copilot

One repository, three modules. Each keeps its own address, server folder, compose project and deploy.

| Module | What | Address | Server folder | Deploy |
|---|---|---|---|---|
| `calendar/` | Merged calendar of every mailbox: gateway (`server/web-ts`), agent (`server/agente-ts`), Windows and Android client (`app/`) | `https://calendar.everbright.it` | `/opt/oe-copilot` | `.github/workflows/calendar.yml` |
| `teams-relay/` | Microsoft Teams chats on a phone web app with push: web app, agents, supervisor, relay, fleet (`app/`), Android app (`mobile/`) | `https://teams.everbright.it` | `/opt/teamsrelay` | `.github/workflows/teams-relay.yml` |
| `mcp/` | One MCP for mail, calendars and Teams over the two modules above | `127.0.0.1:8190/mcp` on the server | `/opt/communication-mcp` | `.github/workflows/mcp.yml` |

## Checks and deploy

There is no CI. Before a push to `main`, run the checks of the module that changed:

```bash
scripts/check.sh calendar|teams-relay|mcp|all [--docker]
```

`--docker` adds compose files, Caddy and the images (Docker needed). On Windows run it from Git Bash.

A push to `main` that touches a module deploys that module. `[skip ci]` in the commit subject skips it. Deploy on request:

```bash
gh workflow run calendar.yml --repo Aptul9/OE-Copilot --ref main
gh workflow run teams-relay.yml --repo Aptul9/OE-Copilot --ref main
gh workflow run mcp.yml --repo Aptul9/OE-Copilot --ref main
gh workflow run teams-relay-android.yml --repo Aptul9/OE-Copilot --ref main   # debug APK, artifact of the run
```

Secrets and variables live in the GitHub environments `calendar`, `teams-relay` and `mcp`. Two are repository secrets because two modules use them: `API_KEY` (calendar agent, MCP mail and calendars) and `MCP_TOKEN` (TeamsRelay `/mcp`, MCP Teams tools).

Module details: `calendar/README.md`, `teams-relay/README.md`, `mcp/README.md`.
