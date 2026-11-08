import { z } from "zod";
import { ConfigError, parseEnv } from "@/shared/env";
import type { AccountsConfig } from "./accounts";

// Environment of the supervisor: the container environment (s6 with-contenv), set by the image and by
// docker-compose.yml. Checked once at start, like the agent's.
const Env = z.object({
  // the OE calendar agent of the shared Microsoft accounts (src/supervisor/broker.ts) and its key for this container
  // (TEAMS_BROKER_KEY of the agent): both or none. Which slot runs on which account the web app decides.
  SHARED_BROKER_URL: z.url({ protocol: /^https?$/ }).optional(),
  SHARED_BROKER_KEY: z.string().min(1).optional(),
  CONTROL_SOCKET: z.string().default("/root/run/control.sock"),
  PROFILES_DIR: z.string().default("/profiles"),
  DATA_DIR: z.string().default("/root/data"),
  VAPID_DIR: z.string().default("/root/vapid"),
  FCM_DIR: z.string().default("/root/fcm"),
  PUID: z.coerce.number().int().min(1).default(1000),
  PGID: z.coerce.number().int().min(1).default(1000),
  XDG_RUNTIME_DIR: z.string().default("/config/.XDG"),
  CHROMIUM: z.string().default("/usr/bin/chromium"),
  AGENT_SCRIPT: z.string().default("/app/agent.cjs"),
  LIGHT_SCRIPT: z.string().default("/app/light.cjs"),
});

// passed on as they are, when set. PULSE_RUNTIME_PATH: where the image runs PulseAudio (/defaults/native), the sound of
// the desktop and the microphone Selkies forwards; without it Chromium looks in XDG_RUNTIME_DIR/pulse and finds none
const BROWSER_ENV = ["TZ", "LANG", "LANGUAGE", "LC_ALL", "PULSE_RUNTIME_PATH"];
const AGENT_ENV = ["TZ", "VAPID_SUBJECT", "NTFY_ENABLED", "NTFY_URL", "NTFY_TOPIC", "LIGHT_POLL", "NATIVE_CALLS", "DOMAIN"];

type SupervisorConfig = { socket: string; accounts: AccountsConfig };

export function loadConfig(env: Record<string, string | undefined> = process.env): SupervisorConfig {
  const e = parseEnv(Env, env);
  if (Boolean(e.SHARED_BROKER_URL) !== Boolean(e.SHARED_BROKER_KEY)) throw new ConfigError("SHARED_BROKER_URL and SHARED_BROKER_KEY go together");
  const sharedBroker = e.SHARED_BROKER_URL ? { url: e.SHARED_BROKER_URL, key: e.SHARED_BROKER_KEY! } : undefined;
  const pick = (names: string[]) => Object.fromEntries(names.flatMap((k) => (env[k] ? [[k, env[k]]] : [])));
  return {
    socket: e.CONTROL_SOCKET,
    accounts: {
      sharedBroker,
      profilesDir: e.PROFILES_DIR,
      dataDir: e.DATA_DIR,
      vapidDir: e.VAPID_DIR,
      fcmDir: e.FCM_DIR,
      uid: e.PUID,
      gid: e.PGID,
      chromium: e.CHROMIUM,
      agentScript: e.AGENT_SCRIPT,
      lightScript: e.LIGHT_SCRIPT,
      node: process.execPath,
      cdpBasePort: 9221,
      // Selkies of the image listens there, behind its nginx (/etc/nginx/sites-available/default)
      desktopPort: 8082,
      // the labwc session of the image (/defaults/startwm_wayland.sh): its clients connect to wayland-0
      session: { XDG_RUNTIME_DIR: e.XDG_RUNTIME_DIR, WAYLAND_DISPLAY: "wayland-0", DISPLAY: ":0" },
      browserEnv: pick(BROWSER_ENV),
      agentEnv: pick(AGENT_ENV),
      wlrctl: "wlrctl",
    },
  };
}
