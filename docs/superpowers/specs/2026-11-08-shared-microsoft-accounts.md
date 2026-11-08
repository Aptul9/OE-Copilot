# Shared Microsoft accounts

One guided Microsoft connection from either account screen. The calendar agent keeps mailbox credentials and service choices. Teams retains its own browser grant when a family refresh token cannot cover both services. Both sites render the same account management source from `shared/accounts/`, including connection progress and account removal.

## Live findings (2026-11-08)

- Six Teams refresh credentials and five Outlook Mobile refresh credentials each redeemed Outlook and Teams CSA tokens; mail, calendar and chat GETs answered 200. Direct cross-audience use of an access token answered 401 in both directions.
- The Outlook Mobile refresh credential is FOCI (`foci: "1"`). Teams web and Outlook web SPA refresh tokens redeemed as another client answer `invalid_grant` 70000. Browser cookies can instead authorize a separate service grant without another password prompt when Microsoft permits SSO.
- `authsvc/v1.0/authz` answers 410 `ApiRestricted` on teams.microsoft.com, authsvc.teams.microsoft.com and teams.cloud.microsoft: no region lookup from a token. The Teams gateway answered the same account on `emea` and `amer`; the six live slots are `emea`. A new slot uses `emea`, a slot taken over keeps the region its browser captured.

## Agent (calendar/server/agente-ts)

Records opt in with `shared: { tenantId, objectId, capabilities, origin }`. Origin is `mobile`, `outlook-spa` or `teams-spa`. Refresh credential and per-resource token cache stay in `tokens.json`. A mailbox that never chose (`shared` absent) counts as mail and calendar. Capabilities may be empty: the mailbox stays signed in and nothing uses it.

- `GET /api/token-resource?account=&resource=outlook|csa|ic3|spaces|presence`: `{token}`, `no-store`. Tenant, object id, audience, scopes and expiry are checked before a rotated refresh token is saved. One per-account lock covers calendar reads and every resource. Removal or replacement during a renewal rejects the result.
- `GET /api/auth/shared`: every Microsoft mailbox with `shared`, `status`, `message` and `teams` (the TeamsRelay slot on it), then `teamsOnly` (TeamsRelay slots with only their own browser sign-in) and `teamsAt`.
- `POST /api/auth/shared {account, capabilities}`: tenant and object id come from a freshly renewed token, never from the request; every capability is proven with a forced resource token, a refusal restores the previous state. A change that only removes capabilities asks Microsoft nothing and works with an expired sign-in. `{account, enabled: false}` removes the metadata.
- The agent refuses to start when `TEAMS_BROKER_KEY` equals `API_KEY`.
- `POST /api/auth/shared/teams {accounts, others}`: TeamsRelay's state report, kept in memory, shown while younger than 10 minutes.
- A source whose account has calendar off is skipped by `/api/eventi` and `/api/calendari`, without an error. Mail off drops it from the mail gateway and `/api/token-posta`.
- `TEAMS_BROKER_KEY`: accepted only for `GET/POST /api/auth/shared`, `POST /api/auth/shared/teams` and `GET /api/token-resource` with a chat resource. Shared operations include correlated onboarding requests and session import. The key cannot call the general authentication endpoints or read Outlook tokens, mail or calendars. `API_KEY` keeps full access.

## TeamsRelay (teams-relay)

- The calendar's agent also sits on network `oe-copilot-teams`, which holds nothing else of the calendar. The browsers container joins it (`docker-compose.oe.yml`, added by `deploy/remote-deploy.sh` when `.env` sets `SHARED_BROKER_URL` and the network exists); the gateway stays out of its reach. The web app stays off it: it reaches the list through the supervisor socket (`GET/POST /directory`, `POST /directory/teams`), so the key lives only in the browsers container.
- A rollback of a TeamsRelay deploy restores the previous `.env` with the previous code. `render-env.sh` refuses `SHARED_BROKER_URL` without the key and leaves out a key without the URL (the secret is shared with the calendar).
- `teams_accounts.shared_account` and `shared_region` bind a slot to an account. Each light start the web app sends carries the binding (`POST /accounts/N/light?shared=&region=`); the supervisor keeps it, gives the light agent `SHARED_BROKER_*`, restarts a light agent whose binding changed, and refuses a browser start for a bound slot (409).
- Every minute (`src/lib/shared-accounts.ts`), inside the slot queue: configured shared accounts get a bound light slot. Existing browser identities are preserved, never taken over. Owner-bound commands stop, resume or remove matching slots. Pending onboarding reuses a matching owner profile or allocates one isolated browser. Nothing changes when the directory cannot be read. The state report then returns to the agent.
- Owner: the user of `SHARED_ACCOUNTS_OWNER`, by default `ADMIN_EMAIL`. Only that user sees the list (`GET/POST /api/shared-accounts`); other users of a multi-user server see nothing.
- Settings renders the shared `Management` and `AccountPanel` components. Identities merge case-insensitively across mailboxes and Teams profiles. Each switch changes one service. Remove confirms deletion of saved access and matching owner slots; Microsoft mailbox contents remain unchanged. A wipe drops the supervisor binding. Mail-only profiles retain a verified mailbox identity without running a Teams agent.
- Light agent in broker mode caches each token until five minutes before its expiry and keeps serving a cached token while the agent restarts. It refuses a token of another account and a slot whose saved identity is another person.

## Calendar web (calendar/server/web-ts)

The Account window opens the same shared management screen as TeamsRelay. Provider-specific calendar settings remain secondary. There is no separate Teams-only category. Turning calendar off reloads the calendars.

## Guided onboarding

- `POST /api/auth/shared {operation: "connect", account, capabilities}` creates an address-bound request in `tokens.json.signins.json`. Adding services preserves already enabled services. Switches remain the way to turn services off.
- Existing usable mobile grants are reused first. Otherwise the owner's browser session silently authorizes Outlook Mobile, then Outlook web once if the mobile client is refused. The stored client determines subsequent authorization. Network failures do not select another client or request another login.
- `signin-progress` binds a request to one slot and exposes a validated desktop URL. `signin-import` accepts a session only for that request, address and slot. Tenant, object id, audience, requested permissions and expiry validate before mailbox credentials are replaced. Browser cookies never return to the web app.
- Missing SSO or additional Microsoft confirmation opens the same persistent browser profile. The Outlook tab handles proxy confirmation when needed. NPO uses the restricted Defender continuation; proxy cookie rotation and the Outlook bootstrap context persist for renewal.
- Mail-only profiles can open a sign-in browser without starting Teams. Teams selections complete only after Teams reports healthy. Requests expire after 30 minutes. Blocked requests are not retried automatically. Expired saved sessions get one guided repair request with current service choices.
- Turning a service off cancels pending onboarding, preventing a late authorization result from restoring older choices. Removing an account also cancels pending authorization.
- Deploy compatibility checks reject onboarding against an older agent instead of interpreting it as a capability update.

## Configuration

| Where | Name | Value |
|---|---|---|
| Repository secret | `TEAMS_BROKER_KEY` | random, 64 hex; agent `TEAMS_BROKER_KEY`, TeamsRelay `SHARED_BROKER_KEY` |
| Environment `teams-relay`, variable | `SHARED_BROKER_URL` | `http://oe-copilot-agent:8180` |
| Environment `teams-relay`, variable | `SHARED_ACCOUNTS_URL` | the calendar address |
| Environment `teams-relay`, variable | `SHARED_ACCOUNTS_OWNER`, `OE_NETWORK` | optional |

## Limits

- One `GestoreToken` owns shared refresh; several processes on one `tokens.json` are unsupported.
- SPA refresh tokens are not converted into family credentials. Separate service grants share the browser SSO session instead.
- Teams resources of an Outlook Mobile sign-in are asked as Teams desktop/mobile (`1fec8e78-bce4-4aaf-ab1b-5451cc387264`), a member of the same family of clients (FOCI). Teams calling (`authsvc/v1.0/authz`) answers 410 `ApiRestricted` to an Outlook Mobile token and 200 with the calling endpoints to a Teams one; the chat list (csa) returned 142 chats to the Teams token. The refresh token that comes back still redeems Outlook as Outlook Mobile (mail and calendar 200). Live, 2026-11-08, notarify.io. A cached token of the other app is asked again.
- Outlook web (`outlook-spa`) grants do not establish Teams calling. The existing or newly authenticated Teams browser grant supplies Teams resources and calls. The light agent starts its calling worker only for a Teams application id.
- Capabilities gate features, not token scopes: an Outlook token is valid for mail and calendar alike, so only trusted services get it (full key); the narrow key never does.
