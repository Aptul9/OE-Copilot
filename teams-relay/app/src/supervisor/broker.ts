// Shared Microsoft accounts of the OE calendar agent, which owns their sign-in. The light agent of a slot bound to one
// (the web app binds it, src/lib/shared-accounts.ts) asks the agent for resource tokens of that account
// (/api/token-resource), never a browser or a credential of its own. The web app reaches the agent's list of accounts
// through the supervisor (GET/POST /directory): the address and the key of the agent stay in this container.
// AccountError carries the HTTP status of the control API; used only when a call fails, so the import cycle with
// accounts.ts is never reached at load time
import { AccountError } from "./accounts";
import path from "node:path";
import fs from "node:fs";
import { SessionAuth } from "../agent/light/session-auth";
import { LightToken } from "../agent/light/token";
import { McasSession } from "../agent/light/mcas-session";
import type { MicrosoftSession } from "../agent/light/session-auth";

export type SharedBroker = { url: string; key: string };
// the shared account of a light slot and its chat service region
export type BrokerBinding = { account: string; region: string };

// the chat service regions of Teams are short lowercase words (amer, emea, apac, in, au, ...)
export const REGION = /^[a-z][a-z0-9]{1,15}$/;
const EMAIL = /^[^\s@/?#]{1,128}@[^\s@/?#]{1,253}$/;

// The binding the web app sends with a light start: ?shared=<account>&region=<region>; none without shared
export function bindingOf(query: URLSearchParams): BrokerBinding | null {
  const account = (query.get("shared") ?? "").trim().toLowerCase();
  if (!account) return null;
  const region = query.get("region") || "emea";
  if (!EMAIL.test(account))
    throw new AccountError(400, "shared: not an account address");
  if (!REGION.test(region))
    throw new AccountError(400, "region: not a chat service region");
  return { account, region };
}

// The broker variables of a light agent bound to a shared account: all four, none for a slot of its own
export function brokerEnv(
  binding: BrokerBinding | null | undefined,
  broker: SharedBroker | undefined,
): Record<string, string> {
  if (!broker || !binding) return {};
  return {
    SHARED_BROKER_URL: broker.url,
    SHARED_BROKER_KEY: broker.key,
    SHARED_BROKER_ACCOUNT: binding.account,
    SHARED_BROKER_REGION: binding.region,
  };
}

export interface Directory {
  list(): Promise<unknown>;
  set(body: unknown): Promise<unknown>;
  report(body: unknown): Promise<unknown>;
  connect?(body: { id: string; account: string; slot: number }): Promise<{ ok: boolean; interaction?: boolean; errore?: string; route?: string }>;
}

// Only the fields the agent's routes take go through: an account and its capabilities, or the report of the slots
function setBody(body: unknown) {
  const b = (body ?? {}) as Record<string, unknown>;
  if (typeof b.account !== "string")
    throw new AccountError(400, "account: an address");
  if (b.operation === "connect") return { operation: b.operation, account: b.account, capabilities: b.capabilities };
  if (b.operation === "signin-progress") return { operation: b.operation, account: b.account, id: b.id, slot: b.slot, phase: b.phase, url: b.url, error: b.error };
  if (
    (b.operation === "remove" || b.operation === "chat") &&
    EMAIL.test(b.account)
  )
    return {
      account: b.account,
      operation: b.operation,
      ...(b.operation === "chat" ? { on: b.on } : {}),
    };
  if (b.enabled === false) return { account: b.account, enabled: false };
  if (!Array.isArray(b.capabilities))
    throw new AccountError(400, "capabilities: a list of mail, calendar, chat");
  return { account: b.account, capabilities: b.capabilities };
}

function reportBody(body: unknown) {
  const b = (body ?? {}) as Record<string, unknown>;
  return {
    accounts: Array.isArray(b.accounts) ? b.accounts : [],
    others: Array.isArray(b.others) ? b.others : [],
    ...(Array.isArray(b.results) ? { results: b.results } : {}),
  };
}

// The list of accounts of the OE calendar agent, with the key of this container: the web app never holds it
export function directory(
  broker: SharedBroker,
  fetchFn: typeof fetch = fetch,
  dataDir?: string,
  capture?: (slot: number, account: string) => Promise<MicrosoftSession | null>,
  captureProxy?: (slot: number) => Promise<{ bootstrapUrl: string; cookies: unknown[] } | null>,
): Directory {
  const call = async (path: string, body?: unknown) => {
    let r: Response;
    try {
      r = await fetchFn(new URL(path, broker.url), {
        method: body === undefined ? "GET" : "POST",
        headers: {
          Authorization: `Bearer ${broker.key}`,
          "Content-Type": "application/json",
        },
        body: body === undefined ? undefined : JSON.stringify(body),
        redirect: "error",
        signal: AbortSignal.timeout((body as { operation?: string })?.operation === "signin-import" ? 150_000 : 45_000),
      });
    } catch {
      throw new AccountError(502, "OE calendar agent not reachable");
    }
    const j = (await r.json().catch(() => null)) as unknown;
    if (r.status === 401)
      throw new AccountError(
        502,
        "OE calendar agent refused the key of the browsers container (TEAMS_BROKER_KEY)",
      );
    if (!r.ok && r.status === 400 && (body as { operation?: string })?.operation === "signin-import") return { ok: false, needsProxy: (j as { needsProxy?: boolean })?.needsProxy === true, needsBrowser: (j as { needsBrowser?: boolean })?.needsBrowser === true, errore: (j as { errore?: string })?.errore || "Microsoft sign-in refused" };
    if (!r.ok || j === null || typeof j !== "object")
      throw new AccountError(502, `OE calendar agent answered ${r.status}`);
    return j;
  };
  return {
    list: () => call("/api/auth/shared"),
    set: (body) => call("/api/auth/shared", setBody(body)),
    report: (body) => call("/api/auth/shared/teams", reportBody(body)),
    connect: async body => {
      if (!dataDir || !Number.isInteger(body.slot) || body.slot < 1 || body.slot > 9999 || !EMAIL.test(body.account)) throw new AccountError(400, "Invalid sign-in slot");
      const folder = path.join(dataDir, String(body.slot));
      const session = await capture?.(body.slot, body.account).catch(() => null) ?? SessionAuth.load(path.join(folder, "session.json"));
      if (!session) return { ok: false, interaction: true };
      if (session.loginHint.trim().toLowerCase() !== body.account) throw new AccountError(409, "Microsoft session identity mismatch");
      const token = LightToken.load(path.join(folder, "light-token.json"));
      const proxy = McasSession.load(path.join(folder, "mcas-session.json"));
      const imported = async (proxy?: { bootstrapUrl: string; cookies: unknown[] } | null) => await call("/api/auth/shared", { operation: "signin-import", ...body, sessione: { ...session, ...(proxy ? { proxy: { bootstrapUrl: proxy.bootstrapUrl, cookies: proxy.cookies } } : {}) }, tenant: token?.tenantId || "organizations" }) as { ok: boolean; interaction?: boolean; needsProxy?: boolean; needsBrowser?: boolean; errore?: string; route?: string };
      let result = await imported(proxy);
      if (result.needsProxy || result.needsBrowser) {
        const captured = await captureProxy?.(body.slot).catch(() => null);
        if (!captured) return { ok: false, interaction: true };
        result = await imported(captured);
      }
      if (result.needsBrowser) return { ok: false, interaction: true };
      if (result.ok) { fs.mkdirSync(folder, { recursive: true, mode: 0o700 }); SessionAuth.save(path.join(folder, "session.json"), session); }
      return result;
    },
  };
}
