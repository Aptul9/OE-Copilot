# Desktop on demand Implementation Plan

**Goal:** The remote desktop of `teams-browsers` (Selkies, nginx, compositor, audio) starts when a browser needs it and stops after 10 minutes without one. Idle container memory drops from about 630 MiB to about 530 MiB.

**Architecture:** The image removes the desktop services from the s6-rc boot bundle and drops the supervisor's dependency on them. A `Desktop` controller in the supervisor starts them with `s6-rc -u change` on `signin` and `start`, and stops them with `s6-rc -d change` when no browser has run for `DESKTOP_IDLE_MIN` minutes. `DESKTOP_ON_DEMAND=0` brings the old always-on behaviour back without an image change.

**Tech Stack:** TypeScript (supervisor, vitest), s6-overlay v3 (`s6-rc`), Docker, Caddy.

Spec: `docs/superpowers/specs/2026-11-10-desktop-on-demand-and-web-consolidation.md`, part A (spike numbers and risks are there).

## Global Constraints

- App root: `teams-relay/app`. Commands run there. Tests: `npx vitest run <file>`; full: `npm run lint && npm run typecheck && npm test`. Repository check before a push to `main` (which deploys): `bash scripts/check.sh teams-relay --docker` from the repository root.
- Style of the surrounding files: short `//` comments for the why, no docstrings. No em dash or en dash in code, comments or docs.
- The desktop services are exactly `svc-de svc-xsettingsd svc-dbus` to start (svc-de pulls in xorg, nginx and selkies) and `svc-de svc-xsettingsd svc-dbus svc-selkies svc-nginx svc-xorg svc-pulseaudio` to stop. Verified on the image by the spike.
- `light`, `stop`, `wipe` and `show` never wait for the desktop. Only `signin` and `start` do.
- `s6-rc` is `/command/s6-rc`. Empty `S6_RC` means no s6 (tests, a supervisor started by hand): the desktop is taken as running and only its Wayland socket is waited for.
- No commit until the lead reviews. Branch: `feat/desktop-on-demand`.

## File map

| File | Task | Role |
|---|---|---|
| `src/supervisor/desktop.ts` | 1 | `Desktop` controller, `runCommand` |
| `test/supervisor/desktop.test.ts` | 1 | its tests |
| `src/supervisor/config.ts`, `test/supervisor/config.test.ts` | 2 | `DESKTOP_ON_DEMAND`, `DESKTOP_IDLE_MIN`, `S6_RC`, desktop settings |
| `src/supervisor/server.ts`, `src/supervisor/main.ts` | 2 | `desktop: () => Promise<void>`, idle tick |
| `test/supervisor/server.test.ts`, `broker.test.ts`, `supervisor-process.test.ts` | 2 | adapt to the new option |
| `app/Dockerfile`, `app/docker/browsers/**` | 3 | boot bundle, dependency, microphone script |
| `docker-compose.yml`, `deploy/render-env.sh`, `.github/workflows/teams-relay.yml`, `caddy/Caddyfile` | 3 | switch plumbing, Caddy redirect |
| `app/scripts/lazy-desktop.sh` | 4 | throwaway-container check of the whole path |

## Task 1: Desktop controller

**Files:**
- Create: `teams-relay/app/src/supervisor/desktop.ts`
- Test: `teams-relay/app/test/supervisor/desktop.test.ts`

**Interfaces produced:**
- `runCommand: Run` where `type Run = (file: string, args: string[]) => Promise<void>`
- `type DesktopOptions = { socket: string; s6rc: string | null; microphone: string | null; idleMs: number; log: (line: string) => void; run?: Run; exists?: (file: string) => boolean; now?: () => number; pollMs?: number; waitMs?: number }`
- `class Desktop { constructor(o: DesktopOptions); get running(): boolean; ensure(): Promise<void>; tick(browserRunning: boolean): Promise<void> }`

- [ ] **Step 1: Write the failing tests** in `test/supervisor/desktop.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { Desktop, type DesktopOptions } from "@/supervisor/desktop";

const SOCKET = "/xdg/wayland-0";
const START = "/command/s6-rc -u change svc-de svc-xsettingsd svc-dbus";
const STOP = "/command/s6-rc -d change svc-de svc-xsettingsd svc-dbus svc-selkies svc-nginx svc-xorg svc-pulseaudio";

function setup(over: Partial<DesktopOptions> = {}) {
  const ctx = { calls: [] as string[], present: new Set<string>(), t: 1_000_000, delay: 0, fail: false, failFile: "", holdStop: null as Promise<void> | null };
  const d = new Desktop({
    socket: SOCKET,
    s6rc: "/command/s6-rc",
    microphone: null,
    idleMs: 600_000,
    log: () => undefined,
    run: async (file, args) => {
      ctx.calls.push([file, ...args].join(" "));
      if (ctx.fail) throw new Error("s6-rc failed");
      if (file === ctx.failFile) throw new Error("no pactl");
      if (args[0] === "-u") setTimeout(() => ctx.present.add(SOCKET), ctx.delay);
      if (args[0] === "-d") {
        await ctx.holdStop;
        ctx.present.delete(SOCKET);
      }
    },
    exists: (f) => ctx.present.has(f),
    now: () => ctx.t,
    pollMs: 1,
    waitMs: 500,
    ...over,
  });
  return { d, ctx };
}

describe("desktop", () => {
  it("starts the desktop once for callers that ask together, and answers when the socket is there", async () => {
    const { d, ctx } = setup();
    ctx.delay = 20;
    await Promise.all([d.ensure(), d.ensure(), d.ensure()]);
    expect(ctx.calls).toEqual([START]);
    expect(ctx.present.has(SOCKET)).toBe(true);
    expect(d.running).toBe(true);
  });

  it("does not start it again while it is up", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    await d.ensure();
    expect(ctx.calls).toEqual([START]);
  });

  it("runs the microphone script once the desktop is up, and carries on when it fails", async () => {
    const log = vi.fn();
    const first = setup({ microphone: "/usr/local/bin/desktop-microphone" });
    await first.d.ensure();
    expect(first.ctx.calls).toEqual([START, "/usr/local/bin/desktop-microphone"]);

    const second = setup({ microphone: "/m", log });
    second.ctx.failFile = "/m";
    await second.d.ensure();
    expect(log).toHaveBeenCalledWith("desktop microphone: no pactl");
    expect(second.d.running).toBe(true);
  });

  it("stops it after the idle time without a browser, not before, and a browser resets the clock", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    ctx.t += 9 * 60_000;
    await d.tick(false);
    expect(ctx.calls).toEqual([START]);
    await d.tick(true);
    ctx.t += 9 * 60_000;
    await d.tick(false);
    expect(ctx.calls).toEqual([START]);
    ctx.t += 2 * 60_000;
    await d.tick(false);
    expect(ctx.calls).toEqual([START, STOP]);
    expect(d.running).toBe(false);
  });

  it("starts it again after it was stopped", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    ctx.t += 11 * 60_000;
    await d.tick(false);
    await d.ensure();
    expect(ctx.calls).toEqual([START, STOP, START]);
    expect(d.running).toBe(true);
  });

  it("keeps the desktop when a start is asked while a stop waits its turn", async () => {
    const { d, ctx } = setup();
    await d.ensure();
    ctx.t += 11 * 60_000;
    const stopping = d.tick(false);
    const starting = d.ensure();
    await Promise.all([stopping, starting]);
    expect(ctx.calls).toEqual([START]);
    expect(d.running).toBe(true);
  });

  it("starts the desktop again after a stop in progress ends, for a start asked meanwhile", async () => {
    const { d, ctx } = setup();
    let release = () => {};
    ctx.holdStop = new Promise<void>((resolve) => (release = resolve));
    await d.ensure();
    ctx.t += 11 * 60_000;
    const stopping = d.tick(false);
    await vi.waitFor(() => expect(ctx.calls).toEqual([START, STOP]));
    const starting = d.ensure();
    release();
    await Promise.all([stopping, starting]);
    expect(ctx.calls).toEqual([START, STOP, START]);
    expect(d.running).toBe(true);
  });

  it("stays down and rejects the callers when the start fails, and tries again at the next ask", async () => {
    const { d, ctx } = setup();
    ctx.fail = true;
    await expect(Promise.all([d.ensure(), d.ensure()])).rejects.toThrow("s6-rc failed");
    expect(d.running).toBe(false);
    ctx.fail = false;
    await d.ensure();
    expect(d.running).toBe(true);
  });

  it("never stops with an infinite idle time, or without s6", async () => {
    const keep = setup({ idleMs: Infinity });
    await keep.d.ensure();
    keep.ctx.t += 24 * 3600_000;
    await keep.d.tick(false);
    expect(keep.ctx.calls).toEqual([START]);

    const bare = setup({ s6rc: null });
    bare.ctx.present.add(SOCKET);
    await bare.d.ensure();
    bare.ctx.t += 24 * 3600_000;
    await bare.d.tick(false);
    expect(bare.ctx.calls).toEqual([]);
    expect(bare.d.running).toBe(true);
  });

  it("gives up when the Wayland socket does not come", async () => {
    const d = new Desktop({ socket: "/never", s6rc: null, microphone: null, idleMs: 1, log: () => undefined, exists: () => false, now: Date.now, pollMs: 2, waitMs: 40 });
    await expect(d.ensure()).rejects.toThrow(/did not come up/);
    expect(d.running).toBe(false);
  });
});
```

- [ ] **Step 2: Run** `npx vitest run test/supervisor/desktop.test.ts`, expect FAIL (module missing).
- [ ] **Step 3: Implement** `src/supervisor/desktop.ts`:

```ts
import { execFile } from "node:child_process";
import fs from "node:fs";
import { errorText } from "../agent/log";

// The remote desktop of the image (linuxserver Selkies) as s6-rc services: svc-de pulls in xorg, nginx and selkies; dbus and
// xsettingsd hang on xorg without being dependencies of it
const START = ["svc-de", "svc-xsettingsd", "svc-dbus"];
const ALL = [...START, "svc-selkies", "svc-nginx", "svc-xorg", "svc-pulseaudio"];

export type Run = (file: string, args: string[]) => Promise<void>;

export const runCommand: Run = (file, args) =>
  new Promise((resolve, reject) => {
    execFile(file, args, { timeout: 60_000 }, (e) => (e ? reject(e) : resolve()));
  });

export type DesktopOptions = {
  // the Wayland socket of the session: the desktop is up when it exists
  socket: string;
  // /command/s6-rc; null where there is no s6 (tests, a supervisor started by hand): the desktop is taken as running
  s6rc: string | null;
  // script that makes the virtual microphone, run once the desktop is up; null: none
  microphone: string | null;
  // stop after this long without a browser; Infinity: never
  idleMs: number;
  log: (line: string) => void;
  run?: Run;
  exists?: (file: string) => boolean;
  now?: () => number;
  pollMs?: number;
  waitMs?: number;
};

// The desktop of the accounts, up while a browser needs it. Start and stop go one after the other, so a start asked while
// the desktop goes down waits for it.
export class Desktop {
  private up = false;
  private lastUsed: number;
  private chain: Promise<unknown> = Promise.resolve();
  private readonly exec: Run;
  private readonly exists: (file: string) => boolean;
  private readonly now: () => number;

  constructor(private readonly o: DesktopOptions) {
    this.exec = o.run ?? runCommand;
    this.exists = o.exists ?? fs.existsSync;
    this.now = o.now ?? Date.now;
    this.lastUsed = this.now();
  }

  get running() {
    return this.up;
  }

  // Up when this resolves, whoever else asked at the same time
  ensure(): Promise<void> {
    this.lastUsed = this.now();
    return this.queue(() => this.start());
  }

  // Once a minute, with whether a browser runs: the desktop stops after idleMs without one
  async tick(browserRunning: boolean): Promise<void> {
    if (browserRunning) {
      this.lastUsed = this.now();
      return;
    }
    if (!this.up || !this.o.s6rc || this.now() - this.lastUsed < this.o.idleMs) return;
    await this.queue(() => this.stop());
  }

  private queue(fn: () => Promise<void>): Promise<void> {
    const next = this.chain.then(fn);
    this.chain = next.catch(() => undefined);
    return next;
  }

  private async start() {
    if (this.up) return;
    // s6-rc takes what is already up as it is, so this is also right when the desktop never went down
    if (this.o.s6rc) {
      this.o.log("desktop starting");
      await this.exec(this.o.s6rc, ["-u", "change", ...START]);
    }
    await this.waitForSocket();
    if (this.o.microphone) await this.exec(this.o.microphone, []).catch((e: unknown) => this.o.log(`desktop microphone: ${errorText(e)}`));
    this.up = true;
    this.o.log("desktop up");
  }

  private async stop() {
    // a browser, or a start that came while this waited its turn, keeps the desktop
    if (!this.up || !this.o.s6rc || this.now() - this.lastUsed < this.o.idleMs) return;
    this.o.log(`desktop stopping: no browser for ${Math.round((this.now() - this.lastUsed) / 60_000)} min`);
    await this.exec(this.o.s6rc, ["-d", "change", ...ALL]);
    this.up = false;
  }

  private async waitForSocket() {
    const wait = this.o.waitMs ?? 60_000;
    const end = this.now() + wait;
    while (!this.exists(this.o.socket)) {
      if (this.now() > end) throw new Error(`the desktop did not come up within ${wait / 1000} s`);
      await new Promise((resolve) => setTimeout(resolve, this.o.pollMs ?? 250));
    }
  }
}
```

- [ ] **Step 4: Run** `npx vitest run test/supervisor/desktop.test.ts` and `npm run typecheck`, expect PASS (10 tests).

## Task 2: Config, control server, supervisor main

**Files:**
- Modify: `src/supervisor/config.ts`, `src/supervisor/server.ts`, `src/supervisor/main.ts`
- Modify: `test/supervisor/config.test.ts`, `test/supervisor/server.test.ts`, `test/supervisor/broker.test.ts`, `test/supervisor/supervisor-process.test.ts`

**Interfaces:**
- Consumes: `Desktop`, `DesktopOptions` (Task 1).
- Produces: `loadConfig(env).desktop: { onDemand: boolean; socket: string; s6rc: string | null; microphone: string | null; idleMs: number }`; `controlServer(accounts, { desktop: () => Promise<void>; log; directory? })`.

- [ ] **Step 1: Write the failing tests.**
  - `config.test.ts`: add `import path from "node:path";` and
    ```ts
    it("starts the desktop on demand, after 10 minutes without a browser, unless DESKTOP_ON_DEMAND is 0", () => {
      expect(loadConfig({}).desktop).toEqual({ onDemand: true, socket: path.join("/config/.XDG", "wayland-0"), s6rc: "/command/s6-rc", microphone: null, idleMs: 600_000 });
      expect(loadConfig({ DESKTOP_ON_DEMAND: "0" }).desktop.onDemand).toBe(false);
      expect(loadConfig({ DESKTOP_IDLE_MIN: "3", S6_RC: "" }).desktop).toMatchObject({ idleMs: 180_000, s6rc: null });
      expect(loadConfig({ SELKIES_MICROPHONE_ENABLED: "true" }).desktop.microphone).toBe("/usr/local/bin/desktop-microphone");
      expect(() => loadConfig({ DESKTOP_ON_DEMAND: "yes" })).toThrow(/DESKTOP_ON_DEMAND/);
    });
    ```
  - `server.test.ts`: add `vi` to the vitest import; in `serve()` change the option to `{ desktop: () => desktop, log: () => undefined }`; add
    ```ts
    it("asks for the desktop on a start and a sign-in only: a light start does not wait for it", async () => {
      let asked = 0;
      let open = () => {};
      const gate = new Promise<void>((resolve) => (open = resolve));
      server = controlServer(accounts(), { desktop: () => { asked++; return gate; }, log: () => undefined });
      await listen(server, sock);
      expect((await request("POST", "/accounts/4/light")).status).toBe(204);
      expect(asked).toBe(0);
      const started = request("POST", "/accounts/5/start");
      const signedIn = request("POST", "/accounts/6/signin");
      await vi.waitFor(() => expect(asked).toBe(2));
      open();
      expect((await started).status).toBe(204);
      expect((await signedIn).status).toBe(204);
    });
    ```
  - `broker.test.ts`: the four `desktop: Promise.resolve()` become `desktop: () => Promise.resolve()` (lines 146, 156, 170, 192).
  - `supervisor-process.test.ts`: add `S6_RC: "",` to the `env` object (after `AGENT_SCRIPT: agent,`): the real supervisor then only waits for the `wayland-0` file the test creates.
- [ ] **Step 2: Run** `npx vitest run test/supervisor`, expect FAIL (type and option errors).
- [ ] **Step 3: Implement.**
  - `config.ts`: `import path from "node:path";`; in `Env` add
    ```ts
      // the remote desktop starts when a browser needs it and stops after DESKTOP_IDLE_MIN minutes without one; 0: it comes up
      // with the supervisor and stays (src/supervisor/desktop.ts)
      DESKTOP_ON_DEMAND: z.enum(["0", "1"]).default("1"),
      DESKTOP_IDLE_MIN: z.coerce.number().int().min(1).max(1440).default(10),
      // empty where there is no s6 (tests, a supervisor started by hand): the desktop is taken as running
      S6_RC: z.string().default("/command/s6-rc"),
      SELKIES_MICROPHONE_ENABLED: z.string().optional(),
    ```
    `type SupervisorConfig = { socket: string; accounts: AccountsConfig; desktop: DesktopSettings };` with `type DesktopSettings = { onDemand: boolean; socket: string; s6rc: string | null; microphone: string | null; idleMs: number };` and in the returned object
    ```ts
      desktop: {
        onDemand: e.DESKTOP_ON_DEMAND === "1",
        socket: path.join(e.XDG_RUNTIME_DIR, "wayland-0"),
        s6rc: e.S6_RC || null,
        microphone: e.SELKIES_MICROPHONE_ENABLED === "true" ? "/usr/local/bin/desktop-microphone" : null,
        idleMs: e.DESKTOP_IDLE_MIN * 60_000,
      },
    ```
  - `server.ts`: in `Options` replace the `desktop: Promise<void>` member with
    ```ts
      // brings the desktop session up and resolves when it is: a browser started before it would find no display
      desktop: () => Promise<void>;
    ```
    and `await desktop;` becomes `await desktop();` in both the `signin` and the `start` case.
  - `main.ts`: remove `waitForFile` and the line `const { XDG_RUNTIME_DIR, WAYLAND_DISPLAY } = cfg.accounts.session;`; `import { Desktop } from "./desktop";`; after `const accounts = ...`:
    ```ts
      const d = cfg.desktop;
      const desktop = new Desktop({ socket: d.socket, s6rc: d.s6rc, microphone: d.microphone, idleMs: d.onDemand ? d.idleMs : Infinity, log: say });
      const desktopError = (e: unknown) => say(`desktop: ${errorText(e)}`);
      // a release that keeps the desktop up brings it up with the supervisor
      if (!d.onDemand) desktop.ensure().catch(desktopError);
      setInterval(() => void desktop.tick(accounts.status().some((a) => a.browser.running)).catch(desktopError), 60_000).unref();
    ```
    and the `controlServer` option `desktop: waitForFile(path.join(XDG_RUNTIME_DIR, WAYLAND_DISPLAY))` becomes `desktop: () => desktop.ensure()`. Keep `fs` and `path` imports (still used for the socket directory).
- [ ] **Step 4: Run** `npx vitest run test/supervisor`, `npm run typecheck`, `npx eslint src/supervisor test/supervisor`, expect PASS and clean.

## Task 3: Image, switch plumbing, Caddy

**Files:**
- Delete: `teams-relay/app/docker/browsers/etc/s6-overlay/s6-rc.d/svc-teamsrelay/dependencies.d/svc-de`
- Create: `teams-relay/app/docker/browsers/etc/s6-overlay/s6-rc.d/svc-teamsrelay/dependencies.d/init-services` (empty file)
- Create: `teams-relay/app/docker/browsers/usr/local/bin/desktop-microphone`
- Modify: `teams-relay/app/docker/browsers/etc/s6-overlay/s6-rc.d/svc-teamsrelay/run`, `teams-relay/app/Dockerfile`, `teams-relay/docker-compose.yml`, `teams-relay/deploy/render-env.sh`, `.github/workflows/teams-relay.yml`, `teams-relay/caddy/Caddyfile`

- [ ] **Step 1: `desktop-microphone`** (the block of the old `run`, as a script of its own):

```bash
#!/usr/bin/with-contenv bash
# The microphone of a call answered from the app (docs/design/2026-09-28-answer-calls.md). Selkies forwards the one
# of the viewer into the "input" sink and makes its virtual source SelkiesVirtualMic only once a viewer connects;
# Teams, answered before that, would record from the default source, the sound of the desktop (output.monitor), and
# the microphone on demand, which waits for an application recording from SelkiesVirtualMic, would never be asked for.
# So the virtual source exists from the start and is the default one. Run by the supervisor once the desktop is up
# (src/supervisor/desktop.ts), when SELKIES_MICROPHONE_ENABLED is true.
as_abc() { s6-setuidgid abc "$@"; }
for _ in $(seq 1 30); do
  as_abc pactl list short sinks 2>/dev/null | grep -q "^[0-9]*[[:space:]]input[[:space:]]" && break
  sleep 1
done
if ! as_abc pactl list short sources 2>/dev/null | grep -q "[[:space:]]SelkiesVirtualMic[[:space:]]"; then
  as_abc pactl load-module module-virtual-source source_name=SelkiesVirtualMic master=input.monitor >/dev/null ||
    echo "[teamsrelay] microphone: no virtual source, calls answered from the app have no microphone"
fi
as_abc pactl set-default-source SelkiesVirtualMic 2>/dev/null || true
```

- [ ] **Step 2: `svc-teamsrelay/run`** becomes:

```bash
#!/usr/bin/with-contenv bash
# Supervisor of the Teams accounts: the browser and the agent of each account (app/src/supervisor). The remote desktop is
# not a dependency of it: the supervisor starts the desktop when a browser needs it (src/supervisor/desktop.ts)

exec node /app/supervisor.cjs
```

- [ ] **Step 3: dependency.** Delete `dependencies.d/svc-de`; create the empty `dependencies.d/init-services`.
- [ ] **Step 4: Dockerfile** (`browsers` stage). After `COPY docker/browsers/ /` add
  ```dockerfile
  # The remote desktop starts when a browser needs it (src/supervisor/desktop.ts): out of the boot bundle of s6-rc, still in
  # its database
  RUN cd /etc/s6-overlay/s6-rc.d/user/contents.d && rm -f svc-de svc-selkies svc-nginx svc-xorg svc-pulseaudio svc-xsettingsd svc-dbus
  ```
  and the existing `RUN chmod +x /etc/s6-overlay/s6-rc.d/svc-teamsrelay/run \` becomes `RUN chmod +x /etc/s6-overlay/s6-rc.d/svc-teamsrelay/run /usr/local/bin/desktop-microphone \`.
- [ ] **Step 5: switch plumbing.** `docker-compose.yml`: after `LIGHT_HOST: ${LIGHT_HOST:-1}` add `DESKTOP_ON_DEMAND: ${DESKTOP_ON_DEMAND:-1}`. `deploy/render-env.sh`: add `DESKTOP_ON_DEMAND` after `LIGHT_HOST` in `KEYS`. `.github/workflows/teams-relay.yml`: after `TR_LIGHT_HOST: ${{ vars.LIGHT_HOST }}` add `TR_DESKTOP_ON_DEMAND: ${{ vars.DESKTOP_ON_DEMAND }}`.
- [ ] **Step 6: Caddy.** In `caddy/Caddyfile`, inside the `{$DOMAIN}` block after the last `handle { ... }`:
  ```
  	# the desktop is off while no browser runs: a direct visit lands on the page that starts it
  	handle_errors {
  		@desktopdown {
  			path /desktop/*
  			expression {err.status_code} in [502, 503, 504]
  		}
  		redir @desktopdown /remote 302
  	}
  ```
- [ ] **Step 7: verify.** `bash -n teams-relay/deploy/render-env.sh`; `bash scripts/check.sh teams-relay --docker` (builds the browsers image with the new layers, runs `agent.cjs --check`, `light.cjs --check`, `supervisor.cjs --check`, and `caddy validate` on the Caddyfile). If `caddy validate` rejects the `expression` line, replace the matcher with `@desktopdown path /desktop/*` and `handle_errors @desktopdown { redir /remote 302 }` and validate again.

## Task 4: Throwaway-container check

**Files:**
- Create: `teams-relay/app/scripts/lazy-desktop.sh`

- [ ] **Step 1: write the script** (Docker host only; used on `oe-vm` before the deploy and by hand later):

```bash
#!/bin/bash
# Checks the on-demand desktop in a throwaway container of the browsers image, on a Docker host:
#   scripts/lazy-desktop.sh <image>      image: docker build --target browsers -t <image> app
# It starts one container, drives the real supervisor through its control socket (sign-in of account 99, which starts a
# browser without touching any account), and removes the container. About 4 minutes: the idle stop is waited for.
set -u
image=${1:?usage: $0 <image>}
name=lazy-desktop-check
fail=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fail=1; }
now() { date +%s.%N; }
since() { echo "scale=1; $(now) - $1" | bc; }
svc() { docker exec "$name" s6-rc -a list 2>&1 | grep -E '^svc-' | sort | tr '\n' ' '; }
mem() { docker stats --no-stream --format '{{.MemUsage}}' "$name"; }
ctl() {
  docker exec "$name" node -e "const r=require('http').request({socketPath:'/root/run/control.sock',method:process.argv[1],path:process.argv[2]},x=>{x.resume();x.on('end',()=>console.log(x.statusCode))});r.on('error',e=>{console.log(e.message);process.exit(1)});r.end()" "$1" "$2"
}
trap 'docker rm -f "$name" >/dev/null 2>&1' EXIT
docker rm -f "$name" >/dev/null 2>&1

docker run -d --name "$name" --security-opt seccomp=unconfined --shm-size=1g --tmpfs /profiles:rw,uid=1000,gid=1000,size=512m \
  -e PUID=1000 -e PGID=1000 -e TZ=Europe/Rome -e PIXELFLUX_WAYLAND=true -e CUSTOM_USER=check -e PASSWORD=check \
  -e SUBFOLDER=/desktop/ -e SELKIES_ENCODER=jpeg -e NATIVE_CALLS=0 -e LIGHT_HOST=1 -e DOMAIN=check.invalid \
  -e DESKTOP_ON_DEMAND=1 -e DESKTOP_IDLE_MIN=1 "$image" >/dev/null

t0=$(now)
for _ in $(seq 1 120); do docker exec "$name" node /app/supervisor.cjs status >/dev/null 2>&1 && break; sleep 0.5; done
docker exec "$name" node /app/supervisor.cjs status >/dev/null 2>&1 && ok "supervisor answers after $(since "$t0") s" || bad "supervisor does not answer"
sleep 10
up=$(svc)
[ "$up" = "svc-cron svc-docker svc-teamsrelay svc-watchdog " ] && ok "boot: only $up" || bad "boot services: $up"
echo "     container memory, supervisor only: $(mem)"

t=$(now)
code=$(ctl POST /accounts/99/signin)
[ "$code" = 204 ] && ok "sign-in of account 99 answered 204 after $(since "$t") s" || bad "sign-in answered $code"
up=$(svc)
case "$up" in *svc-de*svc-xorg*) ok "desktop services up";; *) bad "desktop services: $up";; esac
c=$(docker exec "$name" sh -c 'curl -s -o /dev/null -w %{http_code} --max-time 3 http://127.0.0.1:3000/desktop/')
case "$c" in 200|401|301|302) ok "desktop answers (HTTP $c)";; *) bad "desktop answers $c";; esac
docker exec "$name" sh -c 'pgrep -f /usr/lib/chromium/chromium >/dev/null' && ok "Chromium runs" || bad "no Chromium"
echo "     container memory, desktop and browser up: $(mem)"

[ "$(ctl POST /accounts/99/stop)" = 204 ] && ok "account 99 stopped" || bad "stop failed"
t=$(now)
for _ in $(seq 1 240); do case "$(svc)" in *svc-de*) sleep 1;; *) break;; esac; done
case "$(svc)" in *svc-de*) bad "desktop still up after $(since "$t") s";; *) ok "desktop stopped $(since "$t") s after the browser";; esac
[ "$(docker exec "$name" node /app/supervisor.cjs status >/dev/null 2>&1; echo $?)" = 0 ] && ok "supervisor still answers" || bad "supervisor does not answer"
echo "     container memory, desktop down: $(mem)"

t=$(now)
code=$(ctl POST /accounts/99/signin)
[ "$code" = 204 ] && ok "second sign-in answered 204 after $(since "$t") s" || bad "second sign-in answered $code"
ctl POST /accounts/99/stop >/dev/null
ctl POST /accounts/99/wipe >/dev/null
[ "$fail" = 0 ] && echo "all checks passed" || echo "CHECKS FAILED"
exit "$fail"
```

- [ ] **Step 2: run it on `oe-vm`** against an image built from the branch (the VM builds on deploy; for this check build to a throwaway tag, then remove it):
  - `rsync -az --delete --exclude node_modules --exclude .next teams-relay/app/ oe-vm:/var/tmp/tr-desktop-build/`
  - `ssh oe-vm 'cd /var/tmp/tr-desktop-build && docker build -q --target browsers -t tr-desktop-check . && bash scripts/lazy-desktop.sh tr-desktop-check'`
  - Expected: every line `ok`, ending `all checks passed`; boot services `svc-cron svc-docker svc-teamsrelay svc-watchdog`; sign-in 204 in 6 s or less; desktop stop 60 to 130 s after the browser (idle 1 min plus one tick); memory about 100 MiB with the supervisor only, about 200 MiB with the desktop.
  - Then `ssh oe-vm 'docker rmi tr-desktop-check; rm -rf /var/tmp/tr-desktop-build'`.

## Task 5: Verify, deploy, measure (lead, tonight)

- [ ] **Step 1: local gates.** `bash scripts/check.sh teams-relay --docker` green; app suite green.
- [ ] **Step 2: baseline on `oe-vm`.** `docker stats --no-stream teams-browsers`; `supervisor.cjs status` (9 light slots, 0 restarts); all slots `teams=ok`; no call in progress (`in_call` state empty in each `messages.db`).
- [ ] **Step 3: pre-mortem.** The deploy restarts `teams-browsers`: light slots and call workers stop and come back (about 20 to 30 s); a call in progress would drop. Rollback: `gh variable set DESKTOP_ON_DEMAND --env teams-relay --body 0 --repo Aptul9/OE-Copilot` then `gh workflow run TeamsRelay --ref main --repo Aptul9/OE-Copilot` (desktop comes up with the supervisor, as before), or `git revert` of the merge and push.
- [ ] **Step 4: merge and deploy.** PR with an empty body; the merge to `main` runs the TeamsRelay workflow. Wait for the run.
- [ ] **Step 5: after the deploy.**
  - `docker exec teams-browsers s6-rc -a list | grep ^svc-` shows only `svc-cron svc-docker svc-teamsrelay svc-watchdog`.
  - `docker stats --no-stream teams-browsers`: about 530 MiB or lower (baseline minus about 99 MiB).
  - 9 light slots running, restarts 0, every slot `teams=ok`, `call_audio` as in the baseline, 0 `fetch: slow`, `slot exited` or `host crashed` lines in 15 minutes.
  - End to end on the real container with an unused slot number: `POST /accounts/99/signin` through the control socket (as in `lazy-desktop.sh`), desktop up, Chromium running, `/desktop/` answers through the public Caddy for a logged-in user, then `stop` and `wipe` of 99, and the desktop stops within about 12 minutes (idle 10 minutes plus one tick).
- [ ] **Step 6: vault.** Log in `01_Projects/oe-copilot.md`, Traps line for the on-demand desktop and `DESKTOP_ON_DEMAND=0`, decision note, Board card to Done.
