import http from "node:http";
import { appDb, sharedBinding } from "./appdb";
import { config } from "./config";
import { HttpError } from "./http";

type Binding = { account: string; region: string } | null;

// The shared account of a slot, read when its light agent starts: the supervisor gets it with every light start
function bindingOfSlot(n: number): Binding {
  return sharedBinding(appDb(), n);
}

// The supervisor of the browsers container runs the browser and the agent of every account. The web app
// reaches it only through its unix socket, in the volume shared by the two containers: no Docker access.
export interface ControlClient {
  start(n: number): Promise<void>;
  signin?(n: number): Promise<void>;
  // the light agent alone, browser and agent stopped (a light account between the runs of its browser)
  light(n: number): Promise<void>;
  stop(n: number): Promise<void>;
  // deletes the browser profile (the Microsoft session) of a stopped account
  wipe(n: number): Promise<void>;
  // brings the window of the account to the front of the remote desktop
  show(n: number): Promise<boolean>;
}

type Action = "start" | "signin" | "light" | "stop" | "wipe" | "show";

const VERB: Record<Action, string> = { start: "Start", signin: "Open sign-in browser", light: "Start without browser", stop: "Stop", wipe: "Wipe", show: "Show" };

export function controlClient(socket = config.controlSocket, binding: (n: number) => Binding = bindingOfSlot): ControlClient {
  function post(action: Action, n: number): Promise<string> {
    let query = "";
    if (action === "light") {
      const b = binding(n);
      if (b) query = `?${new URLSearchParams({ shared: b.account, region: b.region })}`;
    }
    return new Promise((resolve, reject) => {
      const req = http.request({ socketPath: socket, method: "POST", path: `/accounts/${n}/${action}${query}`, timeout: 40_000 }, (res) => {
        let body = "";
        res.on("data", (d) => (body += d));
        res.on("end", () => {
          const status = res.statusCode ?? 0;
          if (status >= 200 && status < 300) return resolve(body);
          reject(new HttpError(502, `${VERB[action]} of account ${n} failed: ${detail(body) ?? `status ${status}`}`));
        });
      });
      req.on("timeout", () => req.destroy(new Error("no answer within 40 s")));
      req.on("error", (e) => reject(new HttpError(503, `Browsers container not reachable: ${e.message}`)));
      req.end();
    });
  }

  return {
    start: async (n) => void (await post("start", n)),
    signin: async (n) => void (await post("signin", n)),
    light: async (n) => void (await post("light", n)),
    stop: async (n) => void (await post("stop", n)),
    wipe: async (n) => void (await post("wipe", n)),
    show: async (n) => (JSON.parse((await post("show", n)) || "{}") as { shown?: boolean }).shown === true,
  };
}

// The list of shared Microsoft accounts of the OE calendar agent, through the supervisor, which holds the address and
// key of the agent (src/supervisor/broker.ts). null: no broker in the browsers container, or a supervisor of an earlier
// release; the feature is off.
export interface DirectoryClient {
  list(): Promise<unknown | null>;
  set(body: { account: string; capabilities: string[] } | { account: string; enabled: false } | { account: string; operation: "remove" | "chat"; on?: boolean } | { account: string; operation: "connect" | "signin-progress"; capabilities?: string[]; id?: string; phase?: string; slot?: number; url?: string; error?: string }): Promise<unknown | null>;
  connect?(body: { id: string; account: string; slot: number }): Promise<{ ok: boolean; interaction?: boolean; errore?: string; route?: string }>;
  report(body: { accounts: unknown[]; others: unknown[]; results?: { id: string; error?: string }[] }): Promise<unknown | null>;
}

export function directoryClient(socket = config.controlSocket): DirectoryClient {
  function call(method: "GET" | "POST", path: string, body?: unknown): Promise<unknown | null> {
    return new Promise((resolve, reject) => {
      const data = body === undefined ? undefined : JSON.stringify(body);
      const req = http.request(
        { socketPath: socket, method, path, timeout: path === "/directory/connect" ? 180_000 : 50_000, headers: data ? { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(data) } : {} },
        (res) => {
          let text = "";
          res.on("data", (d) => (text += d));
          res.on("end", () => {
            if (res.statusCode === 404) return resolve(null);
            if (res.statusCode !== 200) return reject(new HttpError(502, `Shared accounts not reached: ${detail(text) ?? `status ${res.statusCode}`}`));
            try {
              resolve(JSON.parse(text) as unknown);
            } catch {
              reject(new HttpError(502, "Shared accounts not reached: not JSON"));
            }
          });
        },
      );
      req.on("timeout", () => req.destroy(new Error("no answer within 50 s")));
      req.on("error", (e) => reject(new HttpError(503, `Browsers container not reachable: ${e.message}`)));
      req.end(data);
    });
  }
  return {
    list: () => call("GET", "/directory"),
    set: (body) => call("POST", "/directory", body),
    report: (body) => call("POST", "/directory/teams", body),
    connect: async body => await call("POST", "/directory/connect", body) as { ok: boolean; interaction?: boolean; errore?: string; route?: string },
  };
}

function detail(body: string): string | null {
  try {
    const d = (JSON.parse(body) as { detail?: unknown }).detail;
    return typeof d === "string" ? d : null;
  } catch {
    return null;
  }
}
