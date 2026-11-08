# Shared Microsoft accounts

Agent owns one refresh credential per identity. Resource-specific access tokens. Explicit mail/calendar/chat capabilities with legacy defaults.

## Plan

1. Extend records with capabilities and resource cache.
2. Serialize refresh per account; validate identity, tenant, audience and expiry. Reject replacement/removal during renewal.
3. Authenticated internal broker endpoint and explicit relay account mapping without credential copies or fallback.
4. Integrate metadata with services and account UI.
5. Test renewal, restart, concurrency and isolation; run suites/typechecks.

## Limits

No deployment or credential migration. Existing MCP edits preserved. Nested independent review blocked by subagent depth limit. Parent supplied live Outlook/Teams HTTP 200 evidence; cross-audience token reuse 401.

## Implemented opt-in slice

Agent records opt in with shared: { tenantId, objectId, capabilities, origin }. Origin is exactly mobile, outlook-spa or teams-spa. Refresh credential and resource cache stay in agent tokens.json. Existing records remain legacy. One running GestoreToken instance owns shared refresh; concurrent processes sharing that file are unsupported.

Authenticated GET /api/token-resource?account=<exact account>&resource=<outlook|csa|ic3|spaces|presence> returns { token } with no-store. Tenant/object identity, audience and expiry are validated before rotation persistence. Calendar callers use the same per-account lock. Removal/replacement during renewal rejects the result. Shared refresh failures never use legacy cached-token fallback.

Teams light agents opt in per slot using all three environment values: SHARED_BROKER_URL, SHARED_BROKER_KEY, SHARED_BROKER_ACCOUNT. Partial configuration fails startup. Broker mode never reads local light-token.json or attempts local refresh/session renewal. No deployment or credential migration performed.

Mail/calendar read gateway enforces declared capability based on route. Direct calendar readers reject missing calendar capability. Shared account listing includes capabilities; legacy listing shape stays unchanged.

## Provisioning

Two consents, both explicit. Nothing changes for an account until both are given.

1. Mailbox owner, calendar Account window: Microsoft mailbox panel, "Account condiviso". Picks capabilities (Posta, Calendario, Chat Teams; default Posta and Calendario, chat off) and presses "Attiva account condiviso". Gateway method set_condivisione_microsoft forwards only address and capabilities to agent POST /api/auth/shared. Agent module calendar/server/agente-ts/src/shared-provisioning.ts takes tenant and object id from claims of a freshly renewed Outlook token (GUIDs, token UPN equal to the mailbox), origin from the client the mailbox signed in with (Outlook Mobile, Outlook web, Teams web). Each chosen capability is then proven with a forced resource token (outlook; csa, ic3, spaces, presence for chat); any refusal restores the previous state. A mailbox already shared keeps identity and origin; only capabilities change. "Spegni" removes the metadata, the mailbox returns to its legacy sign-in.
2. Server operator, browsers container: SHARED_BROKER_URL, SHARED_BROKER_KEY, SHARED_BROKER_SLOTS (JSON slot to account and chat service region). Supervisor validates the mapping (slot numbers, addresses, region words, one account per slot) and refuses to start on a partial or invalid one. Only listed slots get the four SHARED_BROKER_* variables, each with its own account and region; never a browser (start answers 409).
3. Slot owner, Teams Settings: a mapped slot offers only Stopped and "Shared Microsoft account". Choosing it (PATCH /api/accounts/N {shared: true}) runs the light agent; Check now and the desktop link answer 409. GET /broker on the control socket gives the web app slot and account only, never URL, key or region.

Light agent in broker mode writes light_token state {broker: true}: web app never schedules a browser run for it, an ask is forgotten. It refuses a token whose UPN is not the mapped account, and a slot whose saved identity is another person.

## Remaining integration

Agent wiring of /api/auth/shared into server.ts and GestoreToken.impostaShared belong to the core owner (see shared-provisioning.ts header). Teams browsers container must reach the calendar agent over the network; no compose network joins the two stacks. Automated credential migration is intentionally excluded. Live Microsoft proof supplied by parent session predates this implementation; local broker HTTP and renewal tests use simulated Microsoft responses. No new live Microsoft claim is made here.

## Validation evidence

Calendar full suite before final hardening: 17 files, 894 tests passed. Final shared broker suite: 10 tests passed, agent typecheck passed. Teams targeted suite: 4 files, 68 passed, 1 skipped; Teams typecheck passed. Full Teams suite exceeded 120-second shell timeout while running existing local/server-link.test.ts; no complete full-suite result. Test log: C:/Users/Antonio/AppData/Local/Temp/opencode/shared-teams-tests.log.
