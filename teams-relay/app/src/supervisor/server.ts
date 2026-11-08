import fs from "node:fs";
import http from "node:http";
import { AccountError, type Accounts } from "./accounts";
import { bindingOf, type Directory } from "./broker";

export type AccountControl = Pick<Accounts, "start" | "light" | "stop" | "wipe" | "show" | "status">;

type Options = {
  // the desktop session is up: a browser started before it would find no display
  desktop: Promise<void>;
  log: (line: string) => void;
  // the list of shared Microsoft accounts of the OE calendar agent; absent without a broker
  directory?: Directory;
};

type Reply = { status: number; body?: unknown };

const ACTION = /^\/accounts\/([^/]+)\/(start|light|stop|wipe|show)$/;
// the body of a directory request: an account and its capabilities, or the report of the slots
const MAX_BODY = 64 * 1024;

// Control API for the web app, HTTP on a unix socket that only root opens:
//   POST /accounts/N/start | light[?shared=<account>&region=<region>] | stop | wipe | show        GET /accounts
//   GET /directory   POST /directory {account, capabilities | enabled: false}   POST /directory/teams {accounts, others}
export function controlServer(accounts: AccountControl, { desktop, log, directory }: Options): http.Server {
  async function handle(method: string, url: string, body: () => Promise<unknown>): Promise<Reply> {
    const [path, query = ""] = url.split("?");
    if (path === "/accounts") return method === "GET" ? { status: 200, body: accounts.status() } : notAllowed;
    if (path === "/directory" || path === "/directory/teams") {
      if (!directory) return { status: 404, body: { detail: "No shared Microsoft accounts: SHARED_BROKER_URL and SHARED_BROKER_KEY are not set" } };
      if (path === "/directory" && method === "GET") return { status: 200, body: await directory.list() };
      if (method !== "POST") return notAllowed;
      return { status: 200, body: path === "/directory" ? await directory.set(await body()) : await directory.report(await body()) };
    }
    const m = ACTION.exec(path);
    if (!m) return { status: 404, body: { detail: "Not found" } };
    if (method !== "POST") return notAllowed;
    const n = /^\d+$/.test(m[1]) ? Number(m[1]) : Number.NaN;
    switch (m[2]) {
      case "start":
        await desktop;
        await accounts.start(n);
        return { status: 204 };
      case "light":
        await accounts.light(n, bindingOf(new URLSearchParams(query)));
        return { status: 204 };
      case "stop":
        await accounts.stop(n);
        return { status: 204 };
      case "wipe":
        await accounts.wipe(n);
        return { status: 204 };
      default:
        return { status: 200, body: { shown: await accounts.show(n) } };
    }
  }

  return http.createServer((req, res) => {
    handle(req.method ?? "", req.url ?? "", () => readJson(req)).then(
      (reply) => send(res, reply),
      (e: unknown) => {
        const status = e instanceof AccountError ? e.status : 500;
        const detail = e instanceof Error ? e.message : String(e);
        if (status >= 500) log(`control: ${req.method} ${req.url} failed: ${detail}`);
        send(res, { status, body: { detail } });
      },
    );
  });
}

const notAllowed: Reply = { status: 405, body: { detail: "Method not allowed" } };

function readJson(req: http.IncomingMessage): Promise<unknown> {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => {
      size += c.length;
      if (size > MAX_BODY) {
        reject(new AccountError(413, "Request body too large"));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}"));
      } catch {
        reject(new AccountError(400, "Request body is not JSON"));
      }
    });
    req.on("error", reject);
  });
}

function send(res: http.ServerResponse, { status, body }: Reply) {
  if (body === undefined) {
    res.writeHead(status).end();
    return;
  }
  res.writeHead(status, { "Content-Type": "application/json" }).end(JSON.stringify(body));
}

// A socket file left by an earlier run would refuse the listen
export async function listen(server: http.Server, socket: string) {
  const posix = process.platform !== "win32";
  if (posix) fs.rmSync(socket, { force: true });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(socket, () => {
      server.off("error", reject);
      resolve();
    });
  });
  if (posix) fs.chmodSync(socket, 0o600);
}
