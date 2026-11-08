# Shared Microsoft accounts

One sign-in per Microsoft account, made once in the calendar, for mail, calendar and Teams. The calendar agent keeps the account list and, per account, which of the three are on. The calendar and TeamsRelay both show that list and both change it; each component follows it.

## Live findings (2026-11-08)

- Six Teams refresh credentials and five Outlook Mobile refresh credentials each redeemed Outlook and Teams CSA tokens; mail, calendar and chat GETs answered 200. Direct cross-audience use of an access token answered 401 in both directions.
- The Outlook Mobile refresh credential is FOCI (`foci: "1"`) and long-lived. A Teams web (SPA) refresh token redeemed with the Outlook Mobile or Teams desktop client id answers `invalid_grant` 70000: a Teams web sign-in cannot become a long-lived credential. The single sign-in therefore lives on the calendar side.
- `authsvc/v1.0/authz` answers 410 `ApiRestricted` on teams.microsoft.com, authsvc.teams.microsoft.com and teams.cloud.microsoft: no region lookup from a token. The Teams gateway answered the same account on `emea` and `amer`; the six live slots are `emea`. A new slot uses `emea`, a slot taken over keeps the region its browser captured.

## Agent (calendar/server/agente-ts)

Records opt in with `shared: { tenantId, objectId, capabilities, origin }`. Origin is `mobile`, `outlook-spa` or `teams-spa`. Refresh credential and per-resource token cache stay in `tokens.json`. A mailbox that never chose (`shared` absent) counts as mail and calendar. Capabilities may be empty: the mailbox stays signed in and nothing uses it.

- `GET /api/token-resource?account=&resource=outlook|csa|ic3|spaces|presence`: `{token}`, `no-store`. Tenant, object id, audience, scopes and expiry are checked before a rotated refresh token is saved. One per-account lock covers calendar reads and every resource. Removal or replacement during a renewal rejects the result.
- `GET /api/auth/shared`: every Microsoft mailbox with `shared`, `status`, `message` and `teams` (the TeamsRelay slot on it), then `teamsOnly` (TeamsRelay slots with only their own browser sign-in) and `teamsAt`.
- `POST /api/auth/shared {account, capabilities}`: tenant and object id come from a freshly renewed token, never from the request; every capability is proven with a forced resource token, a refusal restores the previous state. `{account, enabled: false}` removes the metadata.
- `POST /api/auth/shared/teams {accounts, others}`: TeamsRelay's state report, kept in memory, shown while younger than 10 minutes.
- A source whose account has calendar off is skipped by `/api/eventi` and `/api/calendari`, without an error. Mail off drops it from the mail gateway and `/api/token-posta`.
- `TEAMS_BROKER_KEY`: a second key, accepted only for `GET/POST /api/auth/shared`, `POST /api/auth/shared/teams` and `GET /api/token-resource` with a chat resource. Never Outlook tokens, mail, calendars or sign-ins. `API_KEY` keeps full access.

## TeamsRelay (teams-relay)

- The browsers container joins the calendar's compose network (`docker-compose.oe.yml`, added by `deploy/remote-deploy.sh` when `.env` sets `SHARED_BROKER_URL` and the network exists). The web app stays off it: it reaches the list through the supervisor socket (`GET/POST /directory`, `POST /directory/teams`), so the key lives only in the browsers container.
- `teams_accounts.shared_account` and `shared_region` bind a slot to an account. Each light start the web app sends carries the binding (`POST /accounts/N/light?shared=&region=`); the supervisor keeps it, gives the light agent `SHARED_BROKER_*`, restarts a light agent whose binding changed, and refuses a browser start for a bound slot (409).
- Every minute (`src/lib/shared-accounts.ts`): for each account with Teams on, a running slot of the owner. An existing slot of the owner signed in to the same address is taken over (no second slot pushing the same messages); otherwise a new slot is claimed, wiped and bound. Teams off, or the account gone from the list, stops the slot and keeps its data. Nothing changes when the list cannot be read. Then the state report goes to the agent.
- Owner: the user of `SHARED_ACCOUNTS_OWNER`, by default `ADMIN_EMAIL`. Only that user sees the list (`GET/POST /api/shared-accounts`); other users of a multi-user server see nothing.
- Settings: "Microsoft accounts" card with Mail, Calendar and Teams switches per account, the Teams-only slots, and a link to the calendar for a new account. A bound slot offers Stopped or On; either is Teams off or on for its account. Removing a bound slot turns Teams off first, otherwise the next pass would add it again.
- Light agent in broker mode caches each token until five minutes before its expiry and keeps serving a cached token while the agent restarts. It refuses a token of another account and a slot whose saved identity is another person.

## Calendar web (calendar/server/web-ts)

The mailbox panel of the Account window ("Usata per") has Posta, Calendario and Teams switches applied at once, and the state of the Teams slot. "Solo in Teams" lists the TeamsRelay slots with only their own sign-in. Turning calendar off reloads the calendars.

## Configuration

| Where | Name | Value |
|---|---|---|
| Repository secret | `TEAMS_BROKER_KEY` | random, 64 hex; agent `TEAMS_BROKER_KEY`, TeamsRelay `SHARED_BROKER_KEY` |
| Environment `teams-relay`, variable | `SHARED_BROKER_URL` | `http://oe-copilot-agent:8180` |
| Environment `teams-relay`, variable | `SHARED_ACCOUNTS_URL` | the calendar address |
| Environment `teams-relay`, variable | `SHARED_ACCOUNTS_OWNER`, `OE_NETWORK` | optional |

## Limits

- One `GestoreToken` owns shared refresh; several processes on one `tokens.json` are unsupported.
- A Teams web sign-in cannot be imported as a shared account (FOCI finding above): new accounts are signed in in the calendar.
- Capabilities gate features, not token scopes: an Outlook token is valid for mail and calendar alike, so only trusted services get it (full key); the narrow key never does.
