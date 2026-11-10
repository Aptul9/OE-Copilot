# Light host Implementation Plan

**Goal:** Run every TeamsRelay light slot in one Node process (`node light.cjs --host`) instead of one process per slot, cutting ~80 MB per extra slot.

**Architecture:** The light bundle gains a host mode driven over the `fork` IPC channel by the supervisor. Per-slot module state is isolated with an `AsyncLocalStorage` slot scope. The supervisor gets a `HostedLight` per slot that implements the existing `ProcessLike`, so `Accounts` keeps its logic; `LIGHT_HOST=0` restores one process per slot.

**Tech Stack:** TypeScript, Node 26 (`node:async_hooks`, `node:child_process` fork IPC), esbuild bundles, vitest.

Spec: `docs/superpowers/specs/2026-11-09-light-host-design.md`.

## Global Constraints

- App root: `teams-relay/app`. Commands run there. Tests: `npx vitest run <file>`; full: `npm run lint && npm run typecheck && npm test`.
- Comment density and style match the surrounding file: short `//` lines explaining why, no docstrings.
- Bundle stays one file `dist/light.cjs`; host mode is `node light.cjs --host`. No new bundle, no image change.
- Single-slot mode (`node light.cjs`, `LIGHT_HOST=0`) must behave exactly as today, including its signal handling and unprefixed log lines.
- Do not commit; the lead reviews and commits.

## File map

| File | Task | Role |
|---|---|---|
| `src/agent/light/host-protocol.ts` | 0 (done by lead) | IPC message types |
| `src/agent/slot-scope.ts` | 1 | slot `AsyncLocalStorage`, `inSlot`, `perSlot`, `slotPrefix` |
| `src/agent/log.ts` | 1 | `[N] ` prefix in host mode |
| `src/agent/teams/chatsvc.ts`, `src/agent/jobs/conversation.ts` | 1 | per-slot caches |
| `src/agent/config.ts` | 2 | `env`, `nativeCalls`, `callingHost`, `callAudioUrl` |
| `src/agent/light/slot.ts` | 2 | `startSlot(env, signal)` |
| `src/agent/light/main.ts`, `loop.ts`, `native-calls.ts` | 2 | use config, single-mode signals only |
| `src/agent/light/host.ts` | 3 | `SlotHost` runtime + `runHost()` |
| `src/supervisor/light-host.ts` | 4 | `LightHost`, `HostedLight` |
| `src/supervisor/accounts.ts`, `config.ts` | 4 | `lightHost` switch |
| `docker-compose.yml`, `deploy/render-env.sh`, `.github/workflows/teams-relay.yml` | 4 | `LIGHT_HOST` plumbing |

## Task 0: IPC protocol (lead, done before dispatch)

`src/agent/light/host-protocol.ts`:

```ts
// Messages between the supervisor and the light host (node light.cjs --host) on the IPC channel of fork
export type ToHost = { op: "start"; slot: number; env: Record<string, string> } | { op: "stop"; slot: number };
export type FromHost = { slot: number; event: "started" } | { slot: number; event: "exited"; how: string } | { slot: number; event: "stopped" };
export const HOST_FLAG = "--host";
```

## Task 1: Slot scope, per-slot caches, log prefix

**Files:**
- Create: `src/agent/slot-scope.ts`
- Modify: `src/agent/log.ts`, `src/agent/teams/chatsvc.ts:133-139,373` and every use, `src/agent/jobs/conversation.ts:61,94` and every use
- Test: `test/agent/slot-scope.test.ts`

**Interfaces produced:**
- `inSlot<T>(slot: number, fn: () => T): T`
- `currentSlot(): number` (0 outside any scope)
- `perSlot<T>(make: () => T): () => T`
- `slotPrefix(): string` (`"[N] "` inside a scope, `""` outside)

- [ ] **Step 1: Write the failing tests** in `test/agent/slot-scope.test.ts`:
  - `perSlot` returns the same instance within a slot, different instances across slots 2 and 3, and slot 0's instance outside any scope.
  - Isolation survives interleaving: two `inSlot` calls each `await` several timers/promises and write to `perSlot(() => new Map())()`; each sees only its own writes.
  - `slotPrefix()` is `""` outside and `"[4] "` inside `inSlot(4, ...)`, also after an `await setTimeout`.
  - `format`-level: `log.info("light", "x")` inside `inSlot(5, ...)` prints `[5] light: x` (spy on `console.log`); outside prints `light: x`.
  - Cache leak regression: `markViewing`/`servedByApi` path in `conversation.ts`: setting a chat as saved by API in slot 2 is not visible in slot 3 (`servedByApi("chat")` false in slot 3). Use the exported functions as they exist.
- [ ] **Step 2: Run** `npx vitest run test/agent/slot-scope.test.ts`, expect FAIL (module missing).
- [ ] **Step 3: Implement** `src/agent/slot-scope.ts`:

```ts
import { AsyncLocalStorage } from "node:async_hooks";

// The light slot the current work belongs to, in the light host where every slot shares one process (src/agent/light/host.ts).
// Outside any scope (one slot per process, tests) the slot is 0.
const scope = new AsyncLocalStorage<number>();

export const inSlot = <T>(slot: number, fn: () => T): T => scope.run(slot, fn);
export const currentSlot = () => scope.getStore() ?? 0;
export const slotPrefix = () => {
  const s = scope.getStore();
  return s === undefined ? "" : `[${s}] `;
};

// Module state that belongs to one account: one instance per slot
export function perSlot<T>(make: () => T): () => T {
  const all = new Map<number, T>();
  return () => {
    const slot = currentSlot();
    let v = all.get(slot);
    if (v === undefined) all.set(slot, (v = make()));
    return v;
  };
}
```

  In `log.ts`: `console.log(slotPrefix() + format(...))` and same for `console.error`.
  In `chatsvc.ts`: replace `const convCache = new Map<...>()` with `const convCaches = perSlot(() => new Map<...>())` and each use `convCache.x` with `convCaches().x`; same for `midConvCache`, `lastMsgByConv`, `unresolved`, `optimisticByConv`. `let lastIdbSyncAt = 0` becomes `const idbSync = perSlot(() => ({ at: 0 }))` with `idbSync().at`. `refusedTokens` and `authCache` unchanged. Same pattern in `conversation.ts` for `recentlyReadChats` and `savedByApi`.
- [ ] **Step 4: Run** the new test plus `npx vitest run test/agent` and `npm run typecheck`, expect PASS.

## Task 2: Config, `startSlot`, single-mode signals

**Files:**
- Modify: `src/agent/config.ts`, `src/agent/light/main.ts`, `src/agent/light/loop.ts:282-288`, `src/agent/light/native-calls.ts:39-42`
- Create: `src/agent/light/slot.ts`
- Test: `test/agent/light-config.test.ts`, update `test/agent/light-loop.test.ts` where it sets `process.env.NATIVE_CALLS`

**Interfaces produced:**
- `Config` gains `env: Record<string, string>` (the env record given to `loadConfig`, undefined values dropped), `nativeCalls: boolean` (`NATIVE_CALLS === "1"`), `callingHost: string` (default `/app/calling-live-host.mjs`), `callAudioUrl: string` (`CALL_AUDIO_URL`, else `wss://${DOMAIN}/api/call/audio/socket?a=${slot}`).
- `startSlot(env: Record<string, string | undefined>, signal?: AbortSignal): Promise<void>` in `src/agent/light/slot.ts`: today's `main()` body from `loadConfig` to `await runLight(l, signal)`, with `loadConfig(env)` and `answerable: config.nativeCalls`.

- [ ] **Step 1: Write failing tests** `test/agent/light-config.test.ts`: `loadConfig` with `NATIVE_CALLS: "1"` gives `nativeCalls: true`, `"0"` and absent give false; `callingHost` default and override; `callAudioUrl` from `CALL_AUDIO_URL`, else built from `DOMAIN` and `ACCOUNT`; `env` holds the given keys.
- [ ] **Step 2: Run**, expect FAIL.
- [ ] **Step 3: Implement.**
  - `config.ts`: add to `Env` `NATIVE_CALLS: z.string().optional()`, `CALLING_HOST: z.string().default("/app/calling-live-host.mjs")`, `CALL_AUDIO_URL: z.string().optional()`, `DOMAIN: z.string().optional()`; fill the new fields; `env: Object.fromEntries(Object.entries(env).filter((e): e is [string, string] => e[1] !== undefined))`.
  - `loop.ts` `runLight`: `const nativeAllowed = l.config.nativeCalls;` and register the two `process.once` handlers only when `!signal` (single mode; the host owns signals).
  - `native-calls.ts`: `fork(l.config.callingHost, ...)` with `env: { ...l.config.env, HOST_STATE: ..., CALL_AUDIO_URL: l.config.callAudioUrl, ... }` (other keys unchanged). Wrap the `message` and `exit` listeners with `AsyncResource.bind(...)` (from `node:async_hooks`) so they keep the slot scope.
  - `slot.ts`: move the context building from `main.ts` here as `startSlot`.
  - `main.ts`: keeps `check()`, signal handler and `main().catch`; `main()` becomes `if (process.argv.includes("--check")) return check(); if (process.argv.includes(HOST_FLAG)) return runHost(); ...signals...; await startSlot(process.env);`. `runHost` comes from Task 3; until then leave the host branch out.
- [ ] **Step 4: Run** `npx vitest run test/agent` and `npm run typecheck`, expect PASS.

## Task 3: Light host runtime

**Files:**
- Create: `src/agent/light/host.ts`
- Modify: `src/agent/light/main.ts` (host branch)
- Test: `test/agent/light-host.test.ts` (unit, fake runner), `test/agent/light-host-process.test.ts` (real bundle)

**Interfaces:**
- Consumes: `ToHost`, `FromHost`, `HOST_FLAG` (Task 0); `inSlot` (Task 1); `startSlot` (Task 2); `restartDelay`, `Timing` from `src/supervisor/process.ts`.
- Produces:

```ts
export type SlotRunner = (env: Record<string, string>, signal: AbortSignal) => Promise<void>;
export class SlotHost {
  constructor(run: SlotRunner, send: (m: FromHost) => void, timing?: Timing);
  handle(m: ToHost): Promise<void>;  // start, restart on new env, stop
  shutdown(graceMs?: number): Promise<void>; // abort all, wait their runs (bounded)
  running(): number[];
}
export function runHost(): void; // wires process IPC, signals, uncaught handlers, using startSlot
```

  Behaviour:
  - `start` of a running slot with the same env (compare `JSON.stringify` of sorted entries): no-op. Different env: stop, then start.
  - A run runs as `inSlot(slot, () => run(env, ctrl.signal))`, sends `started` right after it begins.
  - A run that rejects or resolves while not aborted: send `{event:"exited", how}` (`how` = `errorText(e)` or `"returned"`), log `light: slot exited (<how>), starting again in N s` (inside the slot scope so it is prefixed), restart after `restartDelay(upFor, failures, timing)`.
  - `stop`: abort, await the run (errors ignored), clear any pending restart timer, send `stopped`. `stop` of an unknown slot sends `stopped` at once.
  - `runHost()`: `process.on("message", m => void host.handle(m as ToHost))`; `disconnect`, `SIGTERM`, `SIGINT` → `host.shutdown(30_000).then(() => process.exit(0))`; `uncaughtException` / `unhandledRejection` → `log.warn("light", "host crashed: " + errorText(e))`, `process.exit(1)`. Logs `light: host start pid=<pid>` once.

- [ ] **Step 1: Write failing unit tests** (`test/agent/light-host.test.ts`, fake runner that resolves on abort, fake timing `{ minDelayMs: 10, maxDelayMs: 40, stableMs: 1000, stopGraceMs: 200 }`):
  - start 2 and 3 → both runners called with their env, `started` sent for both, `currentSlot()` inside the runner equals its slot.
  - runner of slot 2 throws → `exited` with the message, restarted after the delay, slot 3 untouched (called once).
  - start 2 again with same env → no new run; with a different env → old signal aborted, new run with new env.
  - stop 3 → signal aborted, `stopped` sent, no restart afterwards even if the runner rejects on abort.
  - stop of unknown slot 9 → `stopped`.
  - shutdown → every signal aborted, resolves after runs end; bounded by `graceMs` if a runner never ends.
- [ ] **Step 2: Run**, expect FAIL.
- [ ] **Step 3: Implement** `host.ts`; add `if (process.argv.includes(HOST_FLAG)) return runHost();` in `main.ts` before the single-mode signal handlers.
- [ ] **Step 4: Run unit tests**, expect PASS.
- [ ] **Step 5: Process test** `test/agent/light-host-process.test.ts` following `test/agent/agent-process.test.ts` (build with `build(bundleOptions("light", bundle))`, `NODE_PATH` to app `node_modules`): `fork(bundle, ["--host"], { stdio: ["ignore","pipe","pipe","ipc"] })`; send `start` for slots 2 and 3 with env `ACCOUNT`, `DB_PATH=<tmp>/<n>/messages.db`, `APP_DB=<tmp>/app.db`, `VAPID_PRIVATE`/`VAPID_APPKEY` missing files, `LIGHT_POLL=15`; expect `started` for both, lines `[2] light: start slot=2` and `[3] light: start slot=3`, both `messages.db` files exist; send `stop` 2 → `stopped`, slot 3 lines continue; process `SIGTERM` (skip exit-code assertion on win32 like the agent test) → exits within 5 s. No network to Microsoft is expected without a sign-in; if a slot logs network errors that is fine.
- [ ] **Step 6: Run** `npx vitest run test/agent/light-host-process.test.ts`, expect PASS.

## Task 4: Supervisor side

**Files:**
- Create: `src/supervisor/light-host.ts`
- Modify: `src/supervisor/accounts.ts` (`AccountsConfig.lightHost: boolean`, `Deps.lightHost?: LightHostLike`, `account()`), `src/supervisor/config.ts` (`LIGHT_HOST`), `src/supervisor/main.ts` (nothing if `Accounts` builds the host itself)
- Modify: `teams-relay/docker-compose.yml` (`LIGHT_HOST: ${LIGHT_HOST:-1}` next to `NATIVE_CALLS`), `teams-relay/deploy/render-env.sh` (`LIGHT_HOST` after `NATIVE_CALLS` in `KEYS`), `.github/workflows/teams-relay.yml` (`TR_LIGHT_HOST: ${{ vars.LIGHT_HOST }}` after `TR_NATIVE_CALLS`)
- Test: `test/supervisor/light-host.test.ts`, extend `test/supervisor/accounts.test.ts` and `test/supervisor/config.test.ts`

**Interfaces:**
- Consumes: `ToHost`, `FromHost`, `HOST_FLAG` from `src/agent/light/host-protocol.ts`; `restartDelay`, `Timing`, `Command` from `./process`; `ProcessLike` from `./accounts`.
- Produces:

```ts
// the channel to one host child; the real one forks, tests give a fake
export interface HostChild {
  readonly pid: number | null;
  send(m: ToHost): void;
  kill(sig: NodeJS.Signals): void;
  on(event: "message", f: (m: FromHost) => void): void;
  on(event: "exit", f: (how: string) => void): void;
}
export type SpawnHost = () => HostChild;
export class LightHost {
  constructor(spawn: SpawnHost, log: (line: string) => void, timing?: Timing);
  slot(n: number, command: () => Command): ProcessLike; // name `light-${n}`
}
export function forkHost(file: string, node: string, env: Record<string, string>, output: (line: string) => void): HostChild; // child_process.fork(file, [HOST_FLAG], { execPath: node, env, stdio: ["ignore","pipe","pipe","ipc"], detached: true }), lines via readline, kill = process group like Supervised
```

  Behaviour:
  - `HostedLight.start()`: marks wanted, `LightHost` spawns the child if none (and no restart timer), then sends `{op:"start", slot, env: command().env}`; when the child is already up, sends at once.
  - On spawn, `start` is sent for every wanted slot (fresh `command()` each time).
  - `running` true after `started`, false after `exited`/`stopped`/child exit. `pid` = child pid while running. `lastExit` from `exited.how` or child exit (`code N`/`signal S`). `restarts` +1 on each `started` after the first.
  - `HostedLight.stop()`: unmark, send `stop`, resolve on `stopped`, on child exit, or after `timing.stopGraceMs`. When no slot is wanted anymore, stop the child: `SIGTERM`, `SIGKILL` after `stopGraceMs`.
  - Child exit while slots are wanted: log `supervisor: light host exited (<how>), starting again in N s`, respawn after `restartDelay`.
  - `Accounts`: when `cfg.lightHost`, `light` of `account(n)` is `this.host.slot(n, () => lightCommand(n, this.cfg, this.bindings.get(n)))`; `this.host = deps.lightHost ?? new LightHost(() => forkHost(cfg.lightScript, cfg.node, hostEnv, line => deps.log(line)), say)`. `hostEnv` = `{ PATH, HOME: "/root", NODE_ENV: "production", ...cfg.agentEnv }` (`lightCommand` already gives each slot its full env).
  - Config: `LIGHT_HOST: z.enum(["0","1"]).default("1")` → `accounts.lightHost = e.LIGHT_HOST === "1"`.

- [ ] **Step 1: Write failing tests** `test/supervisor/light-host.test.ts` with a fake `HostChild` recording sends and letting the test emit messages/exit: start of two slots spawns once and sends two `start`; `started` makes `running` true and `pid` the child pid; child exit marks both not running and respawns after the delay, re-sending both `start`; stop resolves on `stopped`, and on grace timeout when the child never answers; last stop kills the child; `exited` sets `lastExit`. In `accounts.test.ts`: with `lightHost: true` and a fake `lightHost` dep, `light(2)` calls the hosted slot and `stop(2)` stops it; with `lightHost: false` behaviour is today's (existing tests keep `lightHost: false` in `cfg`). In `config.test.ts`: default `lightHost` true, `LIGHT_HOST=0` false.
- [ ] **Step 2: Run**, expect FAIL.
- [ ] **Step 3: Implement** as above; compose/render-env/workflow lines.
- [ ] **Step 4: Run** `npx vitest run test/supervisor` and `npm run typecheck`, expect PASS.

## Task 5: V8 flags (lead, on oe-vm)

- [ ] Build `dist/light.cjs`, copy to `/var/tmp/oe-lighthost/` on `oe-vm`, run inside `teams-browsers` with `--host` and 2 synthetic slots (temp data dirs, no sign-in), 3 runs per flag set (default, `--max-semi-space-size=1`, `--optimize-for-size`, both), anonymous memory after 60 s and CPU seconds from `/proc/<pid>/stat`.
- [ ] Keep the set that lowers memory with CPU within +10%. Apply in `forkHost` via `execArgv` and in `native-calls.ts` fork via `execArgv`. None if no set qualifies.

## Task 6: Verification and rollout (lead)

- [ ] `bash scripts/check.sh teams-relay` from repo root, all green.
- [ ] Baseline on `oe-vm`: per-process anon, `docker stats`, slot states (`supervisor.cjs status`), `call_audio` per slot.
- [ ] Commit per task, push branch, PR with empty body, merge to `main` (deploys TeamsRelay).
- [ ] After deploy: one `light.cjs --host`, zero single `light.cjs`, same slots running, calling hosts for the same slots, memory delta recorded, error lines in 15 min compared to baseline.
- [ ] Rollback if any slot is not back: `gh variable set LIGHT_HOST --env teams-relay --body 0 --repo Aptul9/OE-Copilot` then `gh workflow run TeamsRelay --ref main --repo Aptul9/OE-Copilot`.
