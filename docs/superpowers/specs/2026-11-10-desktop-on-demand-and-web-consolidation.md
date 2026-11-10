# Desktop on demand, and what to do about two web apps

Two parts. Part A (remote desktop started only when a browser is needed) is designed and planned for implementation. Part C (one web app) is analysed, with a decision gate before any code.

## Context

Measured on `oe-vm`, 2026-11-10, after the light host went live (`docs/superpowers/specs/2026-11-09-light-host-design.md`).

- Container `teams-browsers` (image `teamsrelay-browsers`, 4.71 GB, base `lscr.io/linuxserver/chromium`): 630 MiB. It runs the light host (9 slots), 3 call workers, the supervisor and the whole remote desktop (Selkies, nginx, labwc, PulseAudio, dbus). `supervisor.cjs status` shows browser and agent off for every slot, 0 Chromium processes.
- It cannot be stopped: the light host, the call workers and the supervisor live in it.
- Summed RSS of the desktop stack: selkies 105 MB, nginx 41, compositor 45, audio and dbus 13 (204 MB). Summed RSS counts shared pages twice; the container-level figure is below.
- A browser is already on demand in the code: `GET /api/desktop/N` calls `browserOnDemand` (`src/lib/light.ts`), which asks the supervisor to `start` the account; `start` and `signin` in `src/supervisor/server.ts` wait for a `desktop` promise (the Wayland socket) before launching Chromium.

## Part A: desktop on demand

### Measured spike

Throwaway container from the same image on `oe-vm` (no volumes, no ports, removed afterwards), then a derived image with two changes: `svc-teamsrelay` no longer depends on `svc-de`, and the desktop services are removed from `/etc/s6-overlay/s6-rc.d/user/contents.d`.

| Step | Result |
|---|---|
| Boot, supervisor answers `status` | 6.0 s |
| Services up after boot (`s6-rc -a list`) | `svc-cron svc-docker svc-teamsrelay svc-watchdog` |
| Container memory, supervisor only | 96.6 MiB |
| `s6-rc -u change svc-de svc-xsettingsd svc-dbus` to `wayland-0` | 3.5 s (second cycle 4.0 s) |
| Same, until nginx answers `/desktop/` (HTTP 401) | 3.8 s |
| Container memory, desktop up | 195.9 MiB |
| `s6-rc -d change` of the whole desktop | 1.8 s, container 103.4 MiB |
| Supervisor during start and stop | still answers |

Without the dependency change the first spike stopped the supervisor together with the desktop, because s6-rc stops dependents. A cold container (everything at boot) reaches the Wayland socket in 19 s.

Container-level cost of the desktop: 99 MiB (195.9 minus 96.6). The 204 MB summed RSS figure overstates it.

### Design

- **Boot bundle.** The image removes `svc-de svc-selkies svc-nginx svc-xorg svc-pulseaudio svc-xsettingsd svc-dbus` from `user/contents.d`. They stay in the compiled s6-rc database and start only on request. `svc-teamsrelay` depends on `init-services` instead of `svc-de`.
- **Desktop controller.** New `src/supervisor/desktop.ts`, class `Desktop`:
  - `ensure()`: starts the desktop once for any number of concurrent callers (`s6-rc -u change svc-de svc-xsettingsd svc-dbus`), waits for the Wayland socket, then runs the virtual-microphone script when `SELKIES_MICROPHONE_ENABLED=true`. Start and stop are serialized.
  - `tick(browserRunning)`: called every 60 s by the supervisor. A browser running resets the idle clock; after `DESKTOP_IDLE_MIN` (default 10) without one the desktop stops (`s6-rc -d change` of all desktop services).
  - Without s6 (`S6_RC` empty: tests, a supervisor run by hand) `ensure()` only waits for the socket, as `waitForFile` does today.
- **Control server.** `desktop` option becomes `() => Promise<void>`; `signin` and `start` call it. `light`, `stop`, `wipe` and `show` do not. Light accounts never wait for the desktop.
- **Microphone script.** The `pactl` block of `svc-teamsrelay/run` moves to `/usr/local/bin/desktop-microphone`, run by the controller after the desktop is up (PulseAudio no longer exists at supervisor start).
- **Caddy.** `/desktop/*` with the desktop down answers 502 from `browsers:3000`. A `handle_errors` block redirects `/desktop/*` 502, 503 and 504 to `/remote`. The normal path (`/api/desktop/N`, `/remote?account=N`) starts the desktop first and is unaffected.
- **Control client timeouts.** `src/lib/control.ts` allows 40 s per account action, so the 4 to 6 extra seconds on the first sign-in fit.
- **Rollback switch.** `DESKTOP_ON_DEMAND=0` (default 1): the supervisor calls `ensure()` at boot and never stops the desktop. Plumbed like `LIGHT_HOST`: compose, `deploy/render-env.sh`, `.github/workflows/teams-relay.yml`, repository variable.
- **Unchanged:** the container healthcheck (`supervisor.cjs status`), the `teams-caddy` route, the image size (4.71 GB), `Accounts`.

Expected result: `teams-browsers` 630 MiB to about 530 MiB while no browser runs; first sign-in about 4 s slower.

Not in scope (variant B): moving the light host and call workers into a small separate container and starting the browser container on demand. It would also take the 4.7 GB image off the always-on path, but starting a container from the supervisor needs the docker socket (root on the host) or a host-side helper.

### Risks

| Risk | Handling |
|---|---|
| `s6-rc` lock contention during container shutdown | `execFile` timeout 60 s; container stop kills after 35 s regardless |
| A stale `wayland-0` after an unclean stop | `ensure()` always runs `s6-rc -u change` (idempotent, about 0.5 s) before trusting the socket; the controller never assumes the desktop is up at start |
| Supervisor restarts while the desktop is up | `up` starts false; the idle stop only runs after the next `ensure()`; nothing is stopped by surprise |
| A user keeps the remote desktop page open with no browser | The desktop stops after the idle time; the next sign-in starts it again |
| Direct visit to `/desktop/` while down | Caddy redirect to `/remote` |

## Part C: one web app for calendar and Teams

### What the surveys found

Two Next.js apps, each with its own Better Auth login and SQLite file, deployed from two workflows to two directories, behind two Caddies that the host's `sslh` (`/etc/sslh.cfg`) selects by SNI.

| | Calendar gateway (`calendar/server/web-ts`) | TeamsRelay web (`teams-relay/app`) |
|---|---|---|
| Container, memory | `oe-copilot-gateway`, about 107 MiB | `teams-webapp`, about 126 MiB |
| Versions | Next 16.3.6, React 19.3.0, Better Auth 1.7.7 | Next 16.4.5, React 19.3.1, Better Auth 1.7.17 |
| Start | `node server.js`, standalone, user 10001 | `node --require ./call-audio.cjs server.js`, standalone, root (no `USER`) |
| Web-only source | about 16.5k lines, 856 tests | about 19.3k lines (the app tree also holds agent, supervisor, relay, fleet) |
| Routes | `/`, `/login`, `/sicurezza`, `/disponibilita`, `/nuova`, `/salute`, `/api/auth/*`, `/api/disponibilita`, `/api/prenota`, `/condividi/*`, `/mcp`, catch-all `/[...proxy]` (50 API methods, `/logos/*`) | `/`, `/login`, `/admin`, `/settings`, `/remote`, `/consent`, `/[slug]` (public status pages), `/healthz`, `/mcp`, `/.well-known/*`, `/api/auth/*`, about 60 API routes, SSE `/api/events`, websockets `/api/call/audio/socket` and `/api/relay/browser/socket` (upgrade handlers installed by the preload) |
| Middleware or proxy | none | none |
| State | `/dati/gateway.db` plus JSON files | `/data/app.db`, `/data/status.db`, `/data/N/messages.db` (shared with the supervisor and the light host) |
| Owner or users | one owner from `ADMIN_*` | admin plus users, `ACCOUNTS_PER_USER` |
| Origin coupling | `APP_URL` (auth, passkey rpID, cookies, share feed URLs), service worker scope `/`, Windows and Android apps store the origin; `MEET_HOST` is a second name served by the same app | `APP_URL` (auth, MCP OAuth issuer, relay token address), service worker scope `/`, Android app stores the origin, push subscriptions bound to the origin |
| Network | `oe-copilot_default` (agent with the full API key) | `teamsrelay_default`; deliberately not on `oe-copilot-teams`, which only the agent and `teams-browsers` share |

Collisions in one process: `/`, `/login`, `/mcp`, `/api/auth/*`, `/sw.js`, `/icon.png`; the calendar catch-all would shadow unknown Teams paths; both apps read `APP_URL`, `BETTER_AUTH_SECRET`, `ADMIN_EMAIL`, `ADMIN_PASSWORD`, `MCP_TOKEN`, `VAPID_*`, `TZ`, `PORT`; one `instrumentation.ts` and one `register()` per Next app; both boots exit the process on a configuration error.

Memory: one process instead of two saves about 100 MiB; one Caddy instead of two about 17 to 21 MiB.

### Corrections to the earlier framing

- `shared/accounts` already single-sources the accounts screen (5 files, 1,082 lines, generated into both apps by `shared/sync.mjs`), and a change under `shared/**` already triggers both workflows (`calendar.yml:9`, `teams-relay.yml:9`). A change is not made twice.
- The 14 shadcn files with the same names in both apps are not byte-identical in any case. `lib/auth.ts`, `lib/session.ts`, `app/layout.tsx`, `app/login/page.tsx` and `LoginForm.tsx` differ. Only `instrumentation.ts` is identical.
- The earlier reason to decline workspaces (a first deploy restarting every container, `teams-browsers` included) no longer holds: with no browser slots a restart costs seconds.

### What a merge would cost

- **Trust boundary.** The gateway holds the agent's full API key (all mailboxes, Gmail, Chat). The Teams web app holds the Teams data volume and the supervisor socket and exposes relay upload endpoints and an OAuth `/mcp`. Today the Teams web app cannot reach the agent. One process holds both.
- **Privileges.** The gateway runs as uid 10001 without capabilities; the Teams web container runs as root because it writes slot databases owned by the root supervisor. One container needs one user.
- **Host plumbing outside the repo.** `/etc/sslh.cfg` routes `calendar.everbright.it`, `meet.everbright.it` and the sslip name to 4445 and everything else to 4443. Two hostnames in one app keep their origins (service workers, push subscriptions and the stored server address of the Windows and Android apps depend on them), so the host rewrite stays and the sslh rules stay.
- **Cutover.** `gateway.db` and `app.db` stay separate (two Better Auth instances in one process, host-keyed). A new deployment unit joins two compose projects and two pipelines.
- **Tests.** 856 plus 742 web tests import route modules by path (`@/app/mcp/route`, `@/app/api/prenota/route`), walk the source tree (`rete-sicura.test.ts`) and mutate `process.env` (`APP_URL`, `APP_DB`, `BETTER_AUTH_SECRET`).

### Variants

| Variant | What | Saves | Cost | Keeps isolation |
|---|---|---|---|---|
| C-lite | Keep two apps. Nothing structural. | 0 | 0 | yes |
| C-iso | One source tree and one image for both sites, started as two containers (`SITE=calendar` or `teams`). Each container keeps its volumes, network, user and database. A proxy gates the routes of the other site. | 0 RAM; one Next, React and Better Auth version, one Dockerfile, one build | Large: move about 36k lines into per-site route trees under distinct prefixes, two root layouts, test paths | yes |
| C-full | One process, two hostnames routed by a `proxy.ts` rewrite, one Caddy | about 120 MiB | Large plus a trust-boundary loss, one container user, a new deployment unit, host `sslh` review | no |

### Recommendation

- Do Part A. It is what was asked (a browser stack only when a browser is needed), it is small, and it has an env rollback.
- Stay with two web apps (C-lite). The repeated work was in the accounts screens, which are already single-sourced; the remaining duplication is boilerplate that rarely changes; the memory gain of a merge is about 100 MiB on a VM with 20 GB free.
- If a single codebase and build is still wanted, take C-iso, not C-full. Tasks for C-iso are written after the decision, because they depend on the route-prefix scheme chosen.

Decision gate C0: C-lite, C-iso or C-full.
