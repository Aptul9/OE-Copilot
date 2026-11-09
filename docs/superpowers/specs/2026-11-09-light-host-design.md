# TeamsRelay light host: every light slot in one Node process

## Context

Measured on `oe-vm` 2026-11-09 21:13-21:20 (`teams-browsers`, `6e3b9df`, 8-9 light slots, 4 with native calls):

| Process | Count | Anonymous memory each |
|---|---|---|
| `light.cjs` | 8 | 75-101 MB |
| `calling-live-host.mjs` | 4 | ~66 MB |
| `supervisor.cjs` | 1 | ~70-90 MB |
| empty `node` | - | ~8 MB private |
| `light.cjs` bundle loaded, no slot running | - | 88 MB (heap used 36 MB) |

Each light slot pays ~80 MB to load its bundle (zod, google-auth-library, web-push, project code). Per-account state is small. Sockets per slot: 2-3, so connections are not the cost. Prod has FCM and VAPID on (`push=true fcm=true`), so lazy-loading those dependencies saves nothing. V8 flags measured on the bare bundle: `--optimize-for-size` 64 MB, `--max-semi-space-size=1` 68 MB, default 79 MB (single runs, noisy).

Owner chose: V8 flags (A) and one light host process for all slots (B). Calling hosts stay one process per account (native WebRTC fault isolation). Rust rewrite rejected: the calling host runs Microsoft's JS calling SDK, so calls would not change.

## Goal

All light slots of the container run in one Node process. Memory of `teams-browsers` with N light slots drops by about (N-1) x 80 MB. Behaviour per slot unchanged: same chats, pushes, commands, calls, status, logs.

## Design

### Host mode of the light bundle

- `node light.cjs --host` runs the light host. `node light.cjs` (no flag) keeps today's single-slot behaviour. Same bundle, no new image file.
- `src/agent/light/slot.ts`: `startSlot(env, signal)` builds the `Light` context from an env record (today's `main()` body) and runs `runLight(l, signal)`. `main.ts` single mode calls it with `process.env`.
- `src/agent/light/host.ts`: IPC loop over the `fork` channel.
  - Supervisor to host: `{ op: "start", slot, env }`, `{ op: "stop", slot }`.
  - Host to supervisor: `{ slot, event: "started" | "exited", how? }`, `{ slot, event: "stopped" }` after a stop completed.
  - A `start` for a running slot with a different env stops it first, then starts it on the new env (binding change).
- Slot guard: a slot whose `startSlot` rejects is reported `exited`, then started again by the host with `restartDelay` (same timing as `Supervised`: 1 s doubling to 60 s, reset after 5 min up). Other slots are untouched.
- `uncaughtException` and `unhandledRejection` in the host: log, exit 1. The supervisor restarts the host and sends `start` again for every slot it wants.
- SIGTERM or IPC disconnect: abort every slot, wait for each `runLight` to return (its native calls get the existing stop and 25 s kill), exit 0; hard exit after 30 s.

### Per-slot isolation inside one process

- `src/agent/slot-scope.ts`: an `AsyncLocalStorage<number>` holding the current slot, `inSlot(slot, fn)`, and `perSlot(make)` returning the instance of the current slot (slot 0 outside any scope, i.e. single mode and tests).
- Module state reached by the light path becomes per slot through `perSlot`: in `src/agent/teams/chatsvc.ts` `convCache`, `midConvCache`, `lastMsgByConv`, `lastIdbSyncAt`, `unresolved`, `optimisticByConv`; in `src/agent/jobs/conversation.ts` `recentlyReadChats`, `savedByApi`. Two accounts in the same chat see the same `19:...@thread.v2` id, so shared maps would leak state between accounts. `refusedTokens` (keyed by token) and `authCache` (WeakMap by page) stay shared.
- Runtime `process.env` reads on the light path move into `Config`: `NATIVE_CALLS` (`main.ts`, `loop.ts`), `CALLING_HOST`, `CALL_AUDIO_URL`, `DOMAIN` (`native-calls.ts`). `Config.env` keeps the slot's env record; `NativeCalls` forks the calling host with `{ ...l.config.env, ... }` instead of `process.env`.
- Logs: in host mode `log` prefixes `[N] ` from the slot scope, so container lines read as today (`[6] light: start ...`). Single mode prints no prefix (the supervisor adds it, as today).
- Signals: the `process.once("SIGTERM"/"SIGINT")` in `runLight` and in `main.ts` only in single mode; the host owns them.
- Callbacks that leave the async context (child process `message` handlers of `NativeCalls`) are bound with `AsyncResource.bind` so their logs and store writes keep the slot.

### Supervisor

- Config `LIGHT_HOST` (`1` default, `0` = one process per slot as today, the rollback switch). Plumbed like `NATIVE_CALLS`: `teams-browsers` environment `LIGHT_HOST: ${LIGHT_HOST:-1}` in `docker-compose.yml`, key in `deploy/render-env.sh`, `TR_LIGHT_HOST: ${{ vars.LIGHT_HOST }}` in `.github/workflows/teams-relay.yml`.
- `src/supervisor/light-host.ts`: `LightHost` owns one supervised child `node light.cjs --host` started with `fork` (IPC), restarted with `restartDelay`, stopped when no slot is wanted. Its output lines are passed through as they are (the host prefixes them).
- `HostedLight implements ProcessLike` per slot: `start()` records the wanted env (from `lightCommand(n, cfg, binding)`, same env as today) and sends `start`; `stop()` sends `stop` and resolves on `stopped` (or on host exit, or after `stopGraceMs`); `running`, `pid` (host pid), `restarts`, `lastExit` from host events. `Accounts.account()` builds `HostedLight` instead of `Supervised` for `light-N` when `LIGHT_HOST=1`. `Accounts` logic is unchanged.
- On host restart, `LightHost` sends `start` for every wanted slot with its current env.

### V8 flags (A)

- Re-measure on `oe-vm` image, 3 runs each, anonymous memory and CPU of the bare bundle and of a host with 2 synthetic slots: default, `--max-semi-space-size=1`, `--optimize-for-size`, both. Keep the flag set that lowers memory without raising CPU by more than 10%; none if no set qualifies.
- Applied to the host command (`args` before the script) and to the calling host fork (`execArgv`).

## Error handling

| Failure | Effect |
|---|---|
| One slot throws out of `runLight` | That slot restarts with backoff; others continue |
| Host crashes (uncaught) | All light slots stop; supervisor restarts host in 1 s, all slots resume |
| Calling host dies | Unchanged: `NativeCalls` retries after 10 s |
| Host does not answer a stop | Supervisor resolves after `stopGraceMs`; slot marked stopped; host keeps running |

## Testing

- Unit: `slot-scope` isolation across interleaved async work; chatsvc and conversation caches do not leak between two slots with the same conversation id; host IPC start/stop/rebind/guard restart with a fake `startSlot`; `HostedLight` state machine against a fake host channel; log prefix in host mode only.
- Process test: real `dist/light.cjs --host` with 2 slots on a fake Teams endpoint where the existing process tests have one, start/stop/kill-host-restarts.
- Full `scripts/check.sh teams-relay`.

## Rollout

- Baseline on `oe-vm` before deploy: anonymous memory per process, `docker stats`, every slot `Teams ok`, `call_audio` 1 on calls slots.
- Deploy: push to `main` touching `teams-relay/**` runs the TeamsRelay deploy workflow.
- After: one `light.cjs --host` process, no `light.cjs` single processes, same slot count with `Teams ok`, calling hosts present for the same slots, container memory lower by the measured amount, 0 new error lines in 15 min.
- Rollback: `gh variable set LIGHT_HOST --env teams-relay --body 0` then `gh workflow run TeamsRelay --ref main`, or `git revert` of the merge and push.

## Out of scope

Merging calling hosts, supervisor footprint, `dockerd` memory (590 MB RSS after 13 days, noted only).
