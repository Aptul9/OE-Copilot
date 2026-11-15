import path from "node:path";
import { describe, expect, it } from "vitest";
import { ConfigError } from "@/shared/env";
import { loadConfig } from "@/supervisor/config";

describe("loadConfig", () => {
  it("has the paths of the browsers image by default", () => {
    const c = loadConfig({});

    expect(c.socket).toBe("/root/run/control.sock");
    expect(c.accounts).toMatchObject({
      profilesDir: "/profiles",
      dataDir: "/root/data",
      vapidDir: "/root/vapid",
      fcmDir: "/root/fcm",
      uid: 1000,
      gid: 1000,
      chromium: "/usr/bin/chromium",
      agentScript: "/app/agent.cjs",
      lightScript: "/app/light.cjs",
      node: process.execPath,
      cdpBasePort: 9221,
      desktopPort: 8082,
      wlrctl: "wlrctl",
      session: { XDG_RUNTIME_DIR: "/config/.XDG", WAYLAND_DISPLAY: "wayland-0", DISPLAY: ":0" },
    });
  });

  it("runs the light slots in one host unless LIGHT_HOST is 0", () => {
    expect(loadConfig({}).accounts.lightHost).toBe(true);
    expect(loadConfig({ LIGHT_HOST: "1" }).accounts.lightHost).toBe(true);
    expect(loadConfig({ LIGHT_HOST: "0" }).accounts.lightHost).toBe(false);
    expect(() => loadConfig({ LIGHT_HOST: "yes" })).toThrow(/LIGHT_HOST/);
  });

  it("runs the light host in Rust unless LIGHT_RUST is 0", () => {
    expect(loadConfig({}).accounts.lightBin).toBe("/app/teamsrelay-light");
    expect(loadConfig({ LIGHT_BIN: "/opt/light" }).accounts.lightBin).toBe("/opt/light");
    expect(loadConfig({ LIGHT_RUST: "0" }).accounts.lightBin).toBeNull();
    expect(() => loadConfig({ LIGHT_RUST: "yes" })).toThrow(/LIGHT_RUST/);
  });

  it("runs the browser agent in Rust unless AGENT_RUST is 0", () => {
    expect(loadConfig({}).accounts.agentBin).toBe("/app/teamsrelay-agent");
    expect(loadConfig({ AGENT_BIN: "/opt/agent" }).accounts.agentBin).toBe("/opt/agent");
    expect(loadConfig({ AGENT_RUST: "0" }).accounts.agentBin).toBeNull();
    expect(() => loadConfig({ AGENT_RUST: "yes" })).toThrow(/AGENT_RUST/);
  });

  it("starts the desktop on demand, after 10 minutes without a browser, unless DESKTOP_ON_DEMAND is 0", () => {
    expect(loadConfig({}).desktop).toEqual({ onDemand: true, socket: path.join("/config/.XDG", "wayland-0"), s6rc: "/command/s6-rc", microphone: null, idleMs: 600_000 });
    expect(loadConfig({ DESKTOP_ON_DEMAND: "0" }).desktop.onDemand).toBe(false);
    expect(loadConfig({ DESKTOP_IDLE_MIN: "3", S6_RC: "" }).desktop).toMatchObject({ idleMs: 180_000, s6rc: null });
    expect(loadConfig({ SELKIES_MICROPHONE_ENABLED: "true" }).desktop.microphone).toBe("/usr/local/bin/desktop-microphone");
    expect(() => loadConfig({ DESKTOP_ON_DEMAND: "yes" })).toThrow(/DESKTOP_ON_DEMAND/);
  });

  it("passes time zone and language to the browsers, time zone and push settings to the agents", () => {
    const c = loadConfig({ TZ: "Europe/Rome", LANG: "en_US.UTF-8", VAPID_SUBJECT: "mailto:a@b.example", NTFY_ENABLED: "1", NTFY_TOPIC: "t", NTFY_URL: "", OTHER: "x" });

    expect(c.accounts.browserEnv).toEqual({ TZ: "Europe/Rome", LANG: "en_US.UTF-8" });
    expect(c.accounts.agentEnv).toEqual({ TZ: "Europe/Rome", VAPID_SUBJECT: "mailto:a@b.example", NTFY_ENABLED: "1", NTFY_TOPIC: "t" });
  });

  it("passes the sound server of the desktop to the browsers: without it Chromium finds no microphone and plays no sound", () => {
    // the image sets PULSE_RUNTIME_PATH=/defaults, where PulseAudio has its socket; XDG_RUNTIME_DIR/pulse is empty
    expect(loadConfig({ PULSE_RUNTIME_PATH: "/defaults" }).accounts.browserEnv).toEqual({ PULSE_RUNTIME_PATH: "/defaults" });
  });

  it("takes the owner of the profiles from the environment", () => {
    expect(loadConfig({ PUID: "1001", PGID: "1002" }).accounts).toMatchObject({ uid: 1001, gid: 1002 });
  });

  it("stops on a wrong value, naming the variable", () => {
    expect(() => loadConfig({ PUID: "zero" })).toThrow(ConfigError);
    expect(() => loadConfig({ PUID: "0" })).toThrow(/PUID/);
  });
});
