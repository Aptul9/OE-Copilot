import http from "node:http";
import { config } from "./config";
import { HttpError } from "./http";

// The supervisor of the browsers container runs the browser and the agent of every account. The web app
// reaches it only through its unix socket, in the volume shared by the two containers: no Docker access.
export interface ControlClient {
  start(n: number): Promise<void>;
  // the light agent alone, browser and agent stopped (a light account between the runs of its browser)
  light(n: number): Promise<void>;
  stop(n: number): Promise<void>;
  // deletes the browser profile (the Microsoft session) of a stopped account
  wipe(n: number): Promise<void>;
  // brings the window of the account to the front of the remote desktop
  show(n: number): Promise<boolean>;
}

type Action = "start" | "light" | "stop" | "wipe" | "show";

const VERB: Record<Action, string> = { start: "Start", light: "Start without browser", stop: "Stop", wipe: "Wipe", show: "Show" };

export function controlClient(socket = config.controlSocket): ControlClient {
  function post(action: Action, n: number): Promise<string> {
    return new Promise((resolve, reject) => {
      const req = http.request({ socketPath: socket, method: "POST", path: `/accounts/${n}/${action}`, timeout: 40_000 }, (res) => {
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
    light: async (n) => void (await post("light", n)),
    stop: async (n) => void (await post("stop", n)),
    wipe: async (n) => void (await post("wipe", n)),
    show: async (n) => (JSON.parse((await post("show", n)) || "{}") as { shown?: boolean }).shown === true,
  };
}

// The slots the operator mapped to a shared Microsoft account in the browsers container (SHARED_BROKER_SLOTS of the
// supervisor), with their account. A supervisor of an earlier release has no such route: none.
export function brokerSlots(socket = config.controlSocket): Promise<Record<string, { account: string }>> {
  return new Promise((resolve, reject) => {
    const req = http.request({ socketPath: socket, method: "GET", path: "/broker", timeout: 10_000 }, (res) => {
      let body = "";
      res.on("data", (d) => (body += d));
      res.on("end", () => {
        if (res.statusCode === 404) return resolve({});
        if (res.statusCode !== 200) return reject(new HttpError(502, `Shared accounts of the browsers container not read: status ${res.statusCode}`));
        try {
          const slots = (JSON.parse(body) as { slots?: unknown }).slots;
          const out: Record<string, { account: string }> = {};
          if (slots && typeof slots === "object") {
            for (const [slot, v] of Object.entries(slots)) {
              const account = (v as { account?: unknown })?.account;
              if (/^[1-9][0-9]*$/.test(slot) && typeof account === "string" && account) out[slot] = { account };
            }
          }
          resolve(out);
        } catch {
          reject(new HttpError(502, "Shared accounts of the browsers container not read: not JSON"));
        }
      });
    });
    req.on("timeout", () => req.destroy(new Error("no answer within 10 s")));
    req.on("error", (e) => reject(new HttpError(503, `Browsers container not reachable: ${e.message}`)));
    req.end();
  });
}

function detail(body: string): string | null {
  try {
    const d = (JSON.parse(body) as { detail?: unknown }).detail;
    return typeof d === "string" ? d : null;
  } catch {
    return null;
  }
}
