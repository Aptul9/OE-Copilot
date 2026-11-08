# Guided Microsoft sign-in

Goal: one guided onboarding flow in the shared account screen, with browser-session reuse and remembered client selection.

## Design

The owner enters an address and chooses services. The agent persists an onboarding request. TeamsRelay matches only that owner's existing browser identity, or starts one new isolated browser. The browser signs in once; its saved Microsoft session authorizes Outlook Mobile silently, falling back once to Outlook web for policy/interaction restrictions. The selected client and rotating browser cookies persist with the mailbox. A subsequent connection uses the saved client instead of repeating the blocked route. Teams keeps its own browser grant from the same session; no SPA refresh-token conversion or slot takeover occurs.

Requests expose progress and a browser link, never credentials. Missing/expired sessions request interaction without repeated automatic login attempts. Transient errors retry; policy refusals remain visible with affected service. Mail/calendar-only selections retain the browser session without turning Teams on. Existing working mail credentials are not replaced until identity, audience, expiry and permissions validate.

## Implementation

- [x] Test session authorization client selection, refusal classification and remembered fallback before implementing it in `calendar/server/agente-ts/src/sessione-web.ts` and `diretto.ts`.
- [x] Test durable onboarding request validation and result correlation, then add restricted operations under `/api/auth/shared`.
- [x] Test owner-isolated browser matching, bounded interaction and session import through the supervisor, then connect the existing sync worker.
- [x] Add the same email/services/progress form to shared account management; both application adapters use the narrow operation.
- [x] Exercise linked HTTP onboarding, restart/renewal, error handling and existing account preservation. Run typechecks, suites and production builds.
- [x] Record production constraints before any deployment; do not claim an unsupported tenant or service works without a real read.

## Evidence

Agent965, calendar2127, Teams1819 tests passed. Typechecks and production builds passed. Linked broker/agent tests cover ownership, re-add, resume, cancellation and renewal. Current code exercised real MSC and NPO authorization through private scratch stores; mail and calendar returned200 before and after forced-expired refresh recovery. Existing production account state remained unchanged during proof. Fresh interactive MFA through the UI was not observed because the Chrome extension was unavailable.
