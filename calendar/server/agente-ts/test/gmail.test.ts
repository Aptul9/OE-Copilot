import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { GmailGateway } from "../src/gmail.ts";

const ACCOUNT = "me@example.com";
const dirs: string[] = [];
const response = (v: unknown, status = 200, headers = {}) => new Response(JSON.stringify(v), { status, headers });
function setup(fetchFn: typeof fetch, configured = true) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oe-gmail-")); dirs.push(dir);
  const file = path.join(dir, "gmail.json");
  fs.writeFileSync(file, JSON.stringify({ accounts: configured ? { [ACCOUNT]: { provider: "google", client_id: "client", refresh_token: "refresh", access_token: "old", expires_at: Math.floor(Date.now() / 1000) + 3600 } } : {} }));
  return { service: new GmailGateway(file, { fetchFn }), file };
}
afterEach(() => { for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true }); });

describe("Gmail credential-owned gateway", () => {
  it("retains pagination and MIME data; reads never change labels", async () => {
    const calls: { url: string; method: string; auth: string | null }[] = [];
    const { service } = setup(async (input, init) => {
      calls.push({ url: String(input), method: init?.method ?? "GET", auth: new Headers(init?.headers).get("authorization") });
      return response(String(input).includes("pageToken=") ? { messages: [{ id: "m2" }] } : { messages: [{ id: "m1" }], nextPageToken: "opaque+page/=" });
    });
    expect(await service.read({ account: ACCOUNT, path: "messages?maxResults=1&q=is%3Aunread" })).toMatchObject({ nextPageToken: "opaque+page/=" });
    expect(await service.read({ account: ACCOUNT, path: "messages?maxResults=1&pageToken=opaque%2Bpage%2F%3D" })).toMatchObject({ messages: [{ id: "m2" }] });
    expect(calls.every(c => c.method === "GET" && c.auth === "Bearer old")).toBe(true);
    expect(service.accounts()).toEqual([{ account: ACCOUNT, provider: "gmail", status: "ok", message: "" }]);
    expect(JSON.stringify(service.accounts())).not.toMatch(/refresh|Bearer|client/);
  });

  it("rejects URLs, settings, other users, traversal, invalid limits before network access", async () => {
    let count = 0;
    const { service } = setup(async () => { count++; return response({}); });
    for (const p of ["https://evil.test/", "users/victim/messages", "../messages", "messages/%2e%2e/profile", "settings/filters", "messages?maxResults=99999", "messages?bad=x", "messages?format=evil", "messages?maxResults=1&maxResults=2"]) {
      await expect(service.read({ account: ACCOUNT, path: p })).rejects.toThrow();
    }
    await expect(service.read({ account: "other@example.com", path: "profile" })).rejects.toThrow();
    expect(count).toBe(0);
  });

  it("refreshes on 401 once, persists rotated refresh token, returns original response", async () => {
    let refresh = 0; let gets = 0;
    const { service, file } = setup(async (input, init) => {
      if (String(input).includes("oauth2.googleapis.com/token")) {
        refresh++; expect(new URLSearchParams(String(init?.body)).get("refresh_token")).toBe("refresh");
        return response({ access_token: "fresh", refresh_token: "rotated", expires_in: 3600 });
      }
      gets++;
      return new Headers(init?.headers).get("authorization") === "Bearer old" ? response({}, 401) : response({ id: "m1", payload: { mimeType: "text/plain", body: { data: "aGVsbG8" } } });
    });
    expect(await service.read({ account: ACCOUNT, path: "messages/m1?format=full" })).toMatchObject({ payload: { body: { data: "aGVsbG8" } } });
    expect(refresh).toBe(1); expect(gets).toBe(2);
    expect(JSON.parse(fs.readFileSync(file, "utf8")).accounts[ACCOUNT].refresh_token).toBe("rotated");
  });

  it("verifies imported gws identity before persistence; mismatch leaves prior account unchanged", async () => {
    let email = "wrong@example.com";
    const { service, file } = setup(async input => String(input).includes("oauth2.googleapis.com/token") ? response({ access_token: "new-token", expires_in: 3600 }) : response({ emailAddress: email }));
    const before = fs.readFileSync(file, "utf8");
    await expect(service.import({ account: ACCOUNT, credentials: { client_id: "client", client_secret: "secret", refresh_token: "new-refresh" } })).rejects.toThrow(/account/i);
    expect(fs.readFileSync(file, "utf8")).toBe(before);
    email = ACCOUNT;
    await expect(service.import({ account: ACCOUNT, credentials: { client_id: "client", client_secret: "secret", refresh_token: "new-refresh" } })).resolves.toEqual({ account: ACCOUNT, provider: "gmail" });
    expect(JSON.parse(fs.readFileSync(file, "utf8")).accounts[ACCOUNT].refresh_token).toBe("new-refresh");
  });

  it("uses PKCE with explicit matching state, independent mail scopes, no calendar scope", async () => {
    const { service } = setup(async input => String(input).includes("oauth2.googleapis.com/token") ? response({ access_token: "new", refresh_token: "r", expires_in: 3600 }) : response({ emailAddress: ACCOUNT }), false);
    const login = service.begin(ACCOUNT);
    const url = new URL(login.url);
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    expect(url.searchParams.get("scope")).toContain("gmail.modify");
    expect(url.searchParams.get("scope")).not.toContain("calendar");
    await expect(service.complete({ code: "http://localhost/?code=auth&state=wrong", state: login.state })).rejects.toThrow(/state/);
    await expect(service.complete({ code: `http://localhost/?code=auth&state=${login.state}`, state: login.state })).resolves.toEqual({ account: ACCOUNT, provider: "gmail" });
  });

  it("requires explicit confirmation, validates raw MIME and never retries ambiguous send", async () => {
    let calls = 0;
    const { service } = setup(async () => { calls++; return response({ secret: "private upstream" }, 503); });
    const raw = Buffer.from("To: self@example.com\r\nSubject: test\r\n\r\nbody").toString("base64url");
    await expect(service.write({ account: ACCOUNT, action: "send", raw })).rejects.toThrow(/confirm/);
    await expect(service.write({ account: ACCOUNT, action: "send", raw: "invalid!", confirm: true })).rejects.toThrow(/raw/);
    expect(calls).toBe(0);
    await expect(service.write({ account: ACCOUNT, action: "send", raw, confirm: true })).rejects.toThrow("Gmail HTTP 503");
    expect(calls).toBe(1);
  });

  it("creates draft, changes labels, trashes/restores; permanent deletion unavailable", async () => {
    const calls: { url: string; method: string; body: unknown }[] = [];
    const { service } = setup(async (input, init) => {
      calls.push({ url: String(input), method: init?.method ?? "GET", body: init?.body ? JSON.parse(String(init.body)) : null });
      return response({ id: "own-draft" });
    });
    const raw = Buffer.from("Subject: private test\r\n\r\ntest").toString("base64url");
    await service.write({ account: ACCOUNT, action: "draft", raw, confirm: true });
    expect(calls[0]?.body).toEqual({ message: { raw } });
    await service.write({ account: ACCOUNT, action: "modify", message_id: "m1", add_labels: ["STARRED"], remove_labels: ["UNREAD"], confirm: true });
    expect(calls[1]?.body).toEqual({ addLabelIds: ["STARRED"], removeLabelIds: ["UNREAD"] });
    for (const action of ["trash", "untrash"]) await service.write({ account: ACCOUNT, action, message_id: "m1", confirm: true });
    await expect(service.write({ account: ACCOUNT, action: "delete", message_id: "m1", confirm: true })).rejects.toThrow();
    expect(calls).toHaveLength(4);
  });

  it("rejects redirects and oversized responses without exposing body or token", async () => {
    let large = false;
    const { service } = setup(async () => large ? new Response("huge", { headers: { "Content-Length": "999999999" } }) : new Response(null, { status: 302, headers: { Location: "https://evil.test/" } }));
    await expect(service.read({ account: ACCOUNT, path: "profile" })).rejects.toThrow("Gmail HTTP 302");
    large = true;
    await expect(service.read({ account: ACCOUNT, path: "profile" })).rejects.toThrow(/size|byte|limit/);
  });

  it("does not overwrite a newly imported grant with an older in-flight refresh", async () => {
    let release!: () => void;
    let started!: () => void;
    const entered = new Promise<void>(r => { started = r; });
    const blocked = new Promise<void>(r => { release = r; });
    const { service, file } = setup(async (input, init) => {
      if (String(input).includes("oauth2.googleapis.com/token")) {
        const token = new URLSearchParams(String(init?.body)).get("refresh_token");
        if (token === "refresh") { started(); await blocked; return response({ access_token: "stale-access", refresh_token: "stale-refresh", expires_in: 3600 }); }
        return response({ access_token: "new-access", expires_in: 3600 });
      }
      if (new Headers(init?.headers).get("authorization") === "Bearer old") return response({}, 401);
      return response({ emailAddress: ACCOUNT });
    });
    const reading = service.read({ account: ACCOUNT, path: "profile" }).catch(error => error);
    await entered;
    await service.import({ account: ACCOUNT, credentials: { client_id: "new-client", refresh_token: "new-grant" } });
    release();
    await reading;
    expect(JSON.parse(fs.readFileSync(file, "utf8")).accounts[ACCOUNT].refresh_token).toBe("new-grant");
  });

  it("bounds OAuth refresh response parsing before storing or using its token", async () => {
    let reads = 0;
    const { service, file } = setup(async input => {
      if (String(input).includes("oauth2.googleapis.com/token")) return response({ access_token: "oversized", expires_in: 3600, padding: "x".repeat(65536) });
      reads++;
      return reads === 1 ? response({}, 401) : response({ emailAddress: ACCOUNT });
    });
    const before = fs.readFileSync(file, "utf8");
    await expect(service.read({ account: ACCOUNT, path: "profile" })).rejects.toMatchObject({ status: 502 });
    expect(reads).toBe(1);
    expect(fs.readFileSync(file, "utf8")).toBe(before);
  });

  it("applies the same OAuth size cap during credential import", async () => {
    let profiles = 0;
    const { service } = setup(async input => {
      if (String(input).includes("oauth2.googleapis.com/token")) return response({ access_token: "oversized", expires_in: 3600, padding: "x".repeat(65536) });
      profiles++; return response({ emailAddress: ACCOUNT });
    }, false);
    await expect(service.import({ account: ACCOUNT, credentials: { client_id: "client", refresh_token: "grant" } })).rejects.toMatchObject({ status: 502 });
    expect(profiles).toBe(0);
  });
});
