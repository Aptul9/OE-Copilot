// Service boundary: OE-Copilot owns credentials; MCP sends only bounded read requests.
import { execFile } from "node:child_process";
import { Outlook, ApiError, type Obj } from "./api.ts";
import { Tokens, selectAccount, type Account } from "./tokens.ts";
import { backendUrl } from "./url.ts";
import { hostRequest, NAME, parseHostReply } from "./oe-host.ts";
const ENDPOINTS = [
  "/api/communication/accounts",
  "/api/communication/read",
  "/api/communication/google/accounts",
  "/api/communication/gmail/read",
  "/api/communication/gmail/write",
  "/api/communication/gchat/read",
  "/api/communication/gchat/write",
];
export type GatewayCall = (path: string, body?: unknown) => Promise<Obj>;

export function httpGateway(
  base: string,
  key: string,
  fetchFn = fetch,
): GatewayCall {
  const u = backendUrl(base);
  if (!u || u.pathname !== "/")
    throw new Error("Invalid communication service URL");
  return async (p, body) => {
    if (!ENDPOINTS.includes(p))
      throw new ApiError("Unknown communication endpoint");
    const r = await fetchFn(new URL(p, u), {
      method: body === undefined ? "GET" : "POST",
      headers: {
        Authorization: `Bearer ${key}`,
        "Content-Type": "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
      redirect: "error",
      signal: AbortSignal.timeout(45_000),
    });
    let payload: Obj;
    try {
      payload = (await r.json()) as Obj;
    } catch {
      throw new ApiError(
        `Communication service returned invalid JSON (HTTP ${r.status})`,
      );
    }
    if (!r.ok)
      throw new ApiError(
        `Communication service HTTP ${r.status}: ${String(payload.errore ?? "request failed").slice(0, 200)}`,
      );
    return payload;
  };
}

/** Over SSH to the OE Copilot host: the request runs on the host (oe-host.ts), with the agent key that never leaves it. */
export function sshGateway(
  host: string,
  container = "oe-copilot-agent",
  port = 3180,
): GatewayCall {
  if (
    !/^[A-Za-z0-9_][\w.@-]*$/.test(host) ||
    !NAME.test(container) ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535
  )
    throw new Error("Invalid communication SSH configuration");
  return (p, body) =>
    new Promise((resolve, reject) => {
      if (!ENDPOINTS.includes(p))
        return reject(new ApiError("Unknown communication endpoint"));
      execFile(
        "ssh",
        ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, hostRequest(container, port, p, body)],
        { timeout: 45_000, maxBuffer: 13_000_000, windowsHide: true },
        (err, stdout) => {
          if (err)
            return reject(
              new ApiError("Communication service SSH request failed"),
            );
          const envelope = parseHostReply(stdout);
          let payload: Obj;
          try {
            if (!envelope) throw new Error("shape");
            payload = JSON.parse(envelope.body);
          } catch {
            return reject(new ApiError("Communication service returned invalid JSON"));
          }
          if (envelope.status < 200 || envelope.status >= 300)
            return reject(
              new ApiError(
                `Communication service HTTP ${envelope.status}: ${String(payload.errore ?? "request failed").slice(0, 200)}`,
              ),
            );
          resolve(payload);
        },
      );
    });
}

export class ServiceOutlook extends Outlook {
  private readonly gateway: GatewayCall;
  private knownAccounts = new Set<string>();
  constructor(gateway: GatewayCall) {
    // Metadata only. Never request or export Microsoft bearer tokens in service mode.
    super(new Tokens(async () => ({ account: [] })));
    this.gateway = gateway;
    this.tokens.accounts = async () => this.metadata();
    this.tokens.pick = async (account) => this.account(account);
  }
  private async metadata(): Promise<Account[]> {
    const r = await this.gateway("/api/communication/accounts");
    if (!Array.isArray(r.accounts))
      throw new ApiError("Invalid communication account list");
    const accounts: Account[] = r.accounts
      .filter(
        (a: unknown): a is Obj =>
          a !== null &&
          typeof a === "object" &&
          typeof (a as Obj).upn === "string" &&
          /^[^\s@]+@[^\s@]+$/.test((a as Obj).upn),
      )
      .map((a: Obj) => ({
        upn: String(a.upn).toLowerCase(),
        status: String(a.status ?? "ok"),
        message: String(a.message ?? ""),
        exp: 0,
        token: "",
      }));
    this.knownAccounts = new Set(accounts.map((a) => a.upn));
    return accounts;
  }
  override async account(account?: string): Promise<Account> {
    const all = await this.metadata();
    return selectAccount(all, account);
  }
  override async request(
    account: string | undefined,
    p: string,
    method = "GET",
    data?: unknown,
    _retry = true,
    extra?: Record<string, string>,
  ): Promise<Obj> {
    if (method !== "GET" || data !== undefined)
      throw new ApiError("Communication service mode supports read-only tools");
    const who =
      account && this.knownAccounts.has(account.toLowerCase())
        ? { upn: account.toLowerCase() }
        : await this.account(account);
    return this.gateway("/api/communication/read", {
      account: who.upn,
      path: p,
      ...(extra?.Prefer ? { prefer: extra.Prefer } : {}),
    });
  }
}
