// Gmail REST and mail-only OAuth storage. Calendar grants and calendar source lists remain unchanged.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { ErroreAccessoScaduto, GestoreToken, GOOGLE_OAUTH_CLIENT_ID, GOOGLE_OAUTH_CLIENT_SECRET, GOOGLE_OAUTH_REDIRECT_URI, GOOGLE_TOKEN_MAX_BYTES, type VoceToken } from "./diretto.ts";
import { ErroreRispostaGrande, leggiTesto, type FetchFn } from "./outlook-rest.ts";

type Data = Record<string, unknown>;
const BASE = "https://gmail.googleapis.com/gmail/v1/users/me/";
const SCOPES = "https://www.googleapis.com/auth/gmail.modify openid email";
const MAX_BYTES = 8 * 1024 * 1024;
const ID = /^[A-Za-z0-9_-]{1,2048}$/;
const QUERY = new Set(["maxResults", "pageToken", "q", "labelIds", "includeSpamTrash", "format", "metadataHeaders", "fields", "startHistoryId", "historyTypes", "labelId"]);

export class GmailError extends Error {
  readonly status: number;
  constructor(message: string, status = 502) { super(message); this.status = status; }
}

function object(value: unknown): Data {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new GmailError("JSON object required", 400);
  return value as Data;
}

function email(value: unknown): string {
  if (typeof value !== "string" || value.length > 320 || !/^[^\s@]+@[^\s@]+$/.test(value)) throw new GmailError("exact account email required", 400);
  return value.toLowerCase();
}

function identifier(value: unknown): string {
  if (typeof value !== "string" || !ID.test(value)) throw new GmailError("invalid Gmail id", 400);
  return value;
}

export function gmailReadPath(value: unknown): string {
  if (typeof value !== "string" || value.length > 8192 || !/^[\x21-\x7e]+$/.test(value) || /[#\\]/.test(value)) throw new GmailError("invalid Gmail path", 400);
  const parts = value.split("?");
  if (parts.length > 2) throw new GmailError("invalid Gmail path", 400);
  const p = parts[0]!;
  const segments = p.split("/");
  const [type, id, attachment, attachmentId] = segments;
  const valid = p === "profile" || p === "history" || ["labels", "messages", "threads", "drafts"].includes(type ?? "") &&
    (segments.length === 1 || segments.length === 2 && ID.test(id ?? "") || type === "messages" && segments.length === 4 && ID.test(id ?? "") && attachment === "attachments" && ID.test(attachmentId ?? ""));
  if (!valid) throw new GmailError("Gmail read path not allowed", 400);
  const query = new URLSearchParams(parts[1] ?? "");
  const seen = new Set<string>();
  for (const [key, v] of query) {
    if (!QUERY.has(key) || (seen.has(key) && key !== "labelIds" && key !== "metadataHeaders" && key !== "historyTypes")) throw new GmailError("Gmail query not allowed", 400);
    seen.add(key);
    if (key === "maxResults" && (!/^\d{1,3}$/.test(v) || Number(v) < 1 || Number(v) > 500)) throw new GmailError("maxResults must be 1..500", 400);
    if (key === "format" && !["full", "minimal", "raw", "metadata"].includes(v)) throw new GmailError("invalid Gmail format", 400);
    if (key === "includeSpamTrash" && !["true", "false"].includes(v)) throw new GmailError("invalid includeSpamTrash", 400);
    if (/[\x00-\x1f\x7f]/.test(v)) throw new GmailError("invalid Gmail query value", 400);
  }
  return p + (query.size ? `?${query}` : "");
}

export class GmailGateway {
  readonly file: string;
  readonly tokens: GestoreToken;
  private readonly fetchFn: FetchFn;
  private readonly now: () => number;
  private readonly pending = new Map<string, { verifier: string; account?: string; created: number }>();

  constructor(file: string, options: { fetchFn?: FetchFn; now?: () => number } = {}) {
    this.file = file;
    this.fetchFn = options.fetchFn ?? fetch;
    this.now = options.now ?? Date.now;
    this.tokens = new GestoreToken(file, { fetchFn: this.fetchFn, adesso: this.now, strictPersistence: true });
  }

  accounts(): Data[] {
    return Object.entries(this.tokens.elencoAccount()).filter(([account, value]) => value.provider === "google" && !this.tokens.rimosso(account))
      .map(([account, value]) => ({ account, provider: "gmail", status: value.errore ? "errore" : "ok", message: value.errore ? "Gmail sign-in required" : "" }));
  }

  private configured(account: string): void {
    if (!this.accounts().some(a => a.account === account)) throw new GmailError("Gmail account not found", 404);
  }

  private async token(account: string, force: boolean): Promise<string> {
    try {
      const value = await this.tokens.ottieniAccessToken(account, force);
      this.configured(account);
      return value;
    } catch (e) {
      if (e instanceof GmailError) throw e;
      throw new GmailError(e instanceof ErroreAccessoScaduto ? "Gmail sign-in required" : "Gmail token refresh failed", e instanceof ErroreAccessoScaduto ? 409 : 502);
    }
  }

  private async request(account: string, relative: string, method = "GET", body?: unknown): Promise<Data> {
    this.configured(account);
    for (const force of [false, true]) {
      const token = await this.token(account, force);
      let response: Response;
      try { response = await this.fetchFn(`${BASE}${relative}`, { method, headers: { Authorization: `Bearer ${token}`, Accept: "application/json", ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
        body: body === undefined ? undefined : JSON.stringify(body), redirect: "manual", signal: AbortSignal.timeout(30_000) }); }
      catch { throw new GmailError("Gmail transport failed; write outcome may be unknown"); }
      // A rejected 401 is safe to retry. Transport failures, 5xx and decode failures never replay a send.
      if (response.status === 401 && !force) { await response.body?.cancel(); continue; }
      return this.parse(response);
    }
    throw new GmailError("Gmail sign-in required", 409);
  }

  private async parse(response: Response, maxBytes = MAX_BYTES): Promise<Data> {
    if (!response.ok) { await response.body?.cancel(); throw new GmailError(`Gmail HTTP ${response.status}`, [400, 403, 404, 429].includes(response.status) ? response.status : 502); }
    try {
      const text = await leggiTesto(response, maxBytes);
      return text ? object(JSON.parse(text)) : {};
    } catch (e) {
      if (e instanceof ErroreRispostaGrande) throw new GmailError("Gmail response exceeds size limit");
      throw new GmailError("Gmail returned invalid JSON; write outcome may be unknown");
    }
  }

  async read(input: unknown): Promise<Data> {
    const data = object(input);
    if (Object.keys(data).some(k => !["account", "path"].includes(k))) throw new GmailError("unexpected Gmail read field", 400);
    return this.request(email(data.account), gmailReadPath(data.path));
  }

  async write(input: unknown): Promise<Data> {
    const data = object(input);
    if (data.confirm !== true) throw new GmailError("confirm: true required", 400);
    const account = email(data.account);
    const action = data.action;
    if (action === "send" || action === "draft" || action === "update_draft") {
      const fields = ["account", "action", "confirm", "raw", "thread_id", ...(action === "update_draft" ? ["draft_id"] : [])];
      if (Object.keys(data).some(k => !fields.includes(k))) throw new GmailError("unexpected Gmail send field", 400);
      if (typeof data.raw !== "string" || data.raw.length > MAX_BYTES || !/^[A-Za-z0-9_-]+={0,2}$/.test(data.raw)) throw new GmailError("raw must be base64url RFC2822 MIME within size limit", 400);
      const message = { raw: data.raw, ...(data.thread_id ? { threadId: identifier(data.thread_id) } : {}) };
      if (action === "update_draft") {
        const id = identifier(data.draft_id);
        return this.request(account, `drafts/${id}`, "PUT", { id, message });
      }
      return this.request(account, action === "send" ? "messages/send" : "drafts", "POST", action === "send" ? message : { message });
    }
    if (action === "send_draft") {
      if (Object.keys(data).some(k => !["account", "action", "confirm", "draft_id"].includes(k))) throw new GmailError("unexpected draft field", 400);
      return this.request(account, "drafts/send", "POST", { id: identifier(data.draft_id) });
    }
    if (action === "delete_draft") {
      if (Object.keys(data).some(k => !["account", "action", "confirm", "draft_id"].includes(k))) throw new GmailError("unexpected draft field", 400);
      return this.request(account, `drafts/${identifier(data.draft_id)}`, "DELETE");
    }
    if (action === "modify") {
      if (Object.keys(data).some(k => !["account", "action", "confirm", "message_id", "add_labels", "remove_labels"].includes(k))) throw new GmailError("unexpected label field", 400);
      const labels = (value: unknown): string[] => {
        if (value === undefined) return [];
        if (!Array.isArray(value) || value.length > 100 || !value.every(v => typeof v === "string" && ID.test(v))) throw new GmailError("invalid Gmail labels", 400);
        return value as string[];
      };
      const body = { addLabelIds: labels(data.add_labels), removeLabelIds: labels(data.remove_labels) };
      return this.request(account, `messages/${identifier(data.message_id)}/modify`, "POST", body);
    }
    if (action === "trash" || action === "untrash") {
      if (Object.keys(data).some(k => !["account", "action", "confirm", "message_id"].includes(k))) throw new GmailError("unexpected Gmail action field", 400);
      return this.request(account, `messages/${identifier(data.message_id)}/${action}`, "POST", {});
    }
    throw new GmailError("Gmail action not available", 400);
  }

  begin(account?: string): { url: string; state: string; provider: string } {
    for (const [key, value] of this.pending) if (this.now() - value.created > 900_000) this.pending.delete(key);
    if (this.pending.size >= 20) throw new GmailError("too many pending Gmail sign-ins", 429);
    const verifier = crypto.randomBytes(32).toString("base64url");
    const state = crypto.randomBytes(24).toString("base64url");
    this.pending.set(state, { verifier, account: account ? email(account) : undefined, created: this.now() });
    const query = new URLSearchParams({ client_id: GOOGLE_OAUTH_CLIENT_ID, redirect_uri: GOOGLE_OAUTH_REDIRECT_URI, response_type: "code", scope: SCOPES, access_type: "offline", prompt: "consent",
      code_challenge: crypto.createHash("sha256").update(verifier).digest("base64url"), code_challenge_method: "S256", state });
    if (account) query.set("login_hint", email(account));
    return { url: `https://accounts.google.com/o/oauth2/v2/auth?${query}`, state, provider: "gmail" };
  }

  private async exchange(params: Record<string, string>): Promise<Data> {
    let response: Response;
    try { response = await this.fetchFn("https://oauth2.googleapis.com/token", { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body: new URLSearchParams(params).toString(), redirect: "manual", signal: AbortSignal.timeout(15_000) }); }
    catch { throw new GmailError("Gmail OAuth exchange failed"); }
    const result = await this.parse(response, GOOGLE_TOKEN_MAX_BYTES);
    if (typeof result.access_token !== "string" || !result.access_token) throw new GmailError("Gmail OAuth returned no access token");
    return result;
  }

  async complete(input: { code: string; state?: string }): Promise<{ account: string; provider: string }> {
    let code = input.code;
    let state = input.state;
    if (code.includes("://")) {
      let url: URL;
      try { url = new URL(code); } catch { throw new GmailError("invalid OAuth callback", 400); }
      if (url.searchParams.get("error")) throw new GmailError("Gmail consent rejected", 400);
      const given = url.searchParams.get("state");
      if (!given || state && given !== state) throw new GmailError("OAuth state mismatch", 400);
      state = given;
      code = url.searchParams.get("code") ?? "";
    }
    const pending = state ? this.pending.get(state) : undefined;
    if (!pending || this.now() - pending.created > 900_000 || !code) throw new GmailError("OAuth state missing or expired", 400);
    this.pending.delete(state!);
    const token = await this.exchange({ client_id: GOOGLE_OAUTH_CLIENT_ID, client_secret: GOOGLE_OAUTH_CLIENT_SECRET, grant_type: "authorization_code", code, redirect_uri: GOOGLE_OAUTH_REDIRECT_URI, code_verifier: pending.verifier });
    return this.saveVerified(pending.account, { provider: "google", client_id: GOOGLE_OAUTH_CLIENT_ID, client_secret: GOOGLE_OAUTH_CLIENT_SECRET, refresh_token: typeof token.refresh_token === "string" ? token.refresh_token : undefined }, token);
  }

  async import(input: unknown): Promise<{ account: string; provider: string }> {
    const data = object(input);
    const account = email(data.account);
    const credentials = object(data.credentials);
    for (const key of ["client_id", "refresh_token"] as const) if (typeof credentials[key] !== "string" || !credentials[key] || credentials[key].length > 8192) throw new GmailError(`credentials missing ${key}`, 400);
    if (credentials.client_secret !== undefined && (typeof credentials.client_secret !== "string" || credentials.client_secret.length > 2048)) throw new GmailError("invalid OAuth client secret", 400);
    const params = { client_id: credentials.client_id as string, refresh_token: credentials.refresh_token as string, grant_type: "refresh_token", ...(credentials.client_secret ? { client_secret: credentials.client_secret as string } : {}) };
    const token = await this.exchange(params);
    return this.saveVerified(account, { provider: "google", client_id: params.client_id, client_secret: params.client_secret, refresh_token: typeof token.refresh_token === "string" ? token.refresh_token : params.refresh_token }, token);
  }

  private async saveVerified(expected: string | undefined, credentials: VoceToken, token: Data): Promise<{ account: string; provider: string }> {
    const response = await this.fetchFn(`${BASE}profile`, { headers: { Authorization: `Bearer ${String(token.access_token)}`, Accept: "application/json" }, redirect: "manual", signal: AbortSignal.timeout(15_000) });
    const profile = await this.parse(response);
    const account = email(profile.emailAddress);
    if (expected && account !== expected) throw new GmailError("Gmail returned another account", 409);
    const duration = Number(token.expires_in);
    const value: VoceToken = { ...credentials, access_token: String(token.access_token), expires_at: Math.floor(this.now() / 1000) + (Number.isFinite(duration) && duration > 0 ? duration : 3600), scopes: typeof token.scope === "string" ? token.scope : SCOPES };
    const previous = this.tokens.elencoAccount();
    if (!value.refresh_token) value.refresh_token = previous[account]?.refresh_token;
    if (!value.refresh_token) throw new GmailError("Gmail returned no refresh token; consent required", 409);
    const data = { accounts: { ...previous, [account]: value } };
    fs.mkdirSync(path.dirname(this.file), { recursive: true, mode: 0o700 });
    const temporary = `${this.file}.${process.pid}.${crypto.randomBytes(6).toString("hex")}.tmp`;
    fs.writeFileSync(temporary, JSON.stringify(data, null, 2) + "\n", { mode: 0o600 });
    fs.renameSync(temporary, this.file);
    return { account, provider: "gmail" };
  }
}
