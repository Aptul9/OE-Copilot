import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { creaServer } from "../src/server.ts";
import { GoogleCommunication } from "../src/google-communication.ts";
import { encodeChat } from "../src/gchat.ts";

const servers: http.Server[] = [];
const dirs: string[] = [];
afterEach(async () => { for (const s of servers.splice(0)) await new Promise<void>(resolve => s.close(() => resolve())); for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true }); });

it("HTTP agent owns credentials: imports Gmail/Chat, serves reads, confirms writes, preserves calendar accounts", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oe-google-http-")); dirs.push(dir);
  const seen: string[] = [];
  const upstream = http.createServer((req, res) => {
    const parts: Buffer[] = [];
    req.on("data", p => parts.push(p));
    req.on("end", () => {
      const u = new URL(req.url!, "http://fixture"); seen.push(`${req.method} ${u.pathname}`);
      const json = (v: unknown) => { res.setHeader("Content-Type", "application/json"); res.end(JSON.stringify(v)); };
      const proto = (type: string, v: Record<string, unknown>) => res.end(Buffer.from(encodeChat(type, v)));
      if (u.pathname === "/token") return json({ access_token: "mail-private-token", expires_in: 3600 });
      if (u.pathname.endsWith("/profile")) {
        expect(req.headers.authorization).toBe("Bearer mail-private-token");
        return json({ emailAddress: "me@example.com" });
      }
      if (u.pathname.endsWith("/messages")) return json({ messages: [{ id: "m1" }], nextPageToken: "page2" });
      if (u.pathname.endsWith("/mole/world")) return res.end('<script>window.WIZ_global_data = {"qwAQke":"ChatUi","SMqcke":"xsrf"};</script>');
      if (u.pathname.endsWith("/get_self_user_status")) return proto("GetSelfUserStatusResponse", { user_status: { user_id: { id: "123" } } });
      if (u.pathname.endsWith("/get_members")) return proto("GetMembersResponse", { members: [{ user: { user_id: { id: "123" }, name: "Me", email: "me@example.com" } }] });
      if (u.pathname.endsWith("/paginated_world")) return proto("PaginatedWorldResponse", { world_items: [] });
      if (u.pathname.endsWith("/create_topic")) return proto("CreateTopicResponse", { topic: { id: { topic_id: "new" } } });
      res.statusCode = 404; res.end("{}");
    });
  });
  servers.push(upstream);
  await new Promise<void>(resolve => upstream.listen(0, "127.0.0.1", resolve));
  const upstreamBase = `http://127.0.0.1:${(upstream.address() as { port: number }).port}`;
  const bridge: typeof fetch = (input, init) => { const u = new URL(String(input)); return fetch(`${upstreamBase}${u.pathname}${u.search}`, init); };
  const google = new GoogleCommunication(dir, { fetchFn: bridge });
  const { server } = creaServer({ chiave: "key", fonti: [], battito: path.join(dir, "missing"), zona: "Europe/Rome", google, registro: () => {} });
  servers.push(server);
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  const post = async (url: string, body: unknown, key = "key") => {
    const r = await fetch(base + url, { method: "POST", headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" }, body: JSON.stringify(body) });
    return { status: r.status, body: await r.json() as Record<string, unknown> };
  };
  const invalid = await post("/api/auth/completa", { provider: "gmail" }, "wrong");
  expect(invalid.status).toBe(401); expect(seen).toHaveLength(0);
  const gmail = await post("/api/auth/completa", { provider: "gmail", account: "me@example.com", credentials: { client_id: "client", refresh_token: "mail-refresh" } });
  expect(gmail).toMatchObject({ status: 200, body: { ok: true, account: "me@example.com", provider: "gmail" } });
  const chat = await post("/api/auth/completa", { provider: "gchat", account: "me@example.com", sessione: { user_agent: "test", cookies: [{ name: "SID", value: "private-cookie", domain: ".google.com", path: "/", expires: 0 }] } });
  expect(chat).toMatchObject({ status: 200, body: { ok: true, provider: "gchat" } });
  const accounts = await (await fetch(base + "/api/communication/google/accounts", { headers: { Authorization: "Bearer key" } })).text();
  expect(JSON.parse(accounts).accounts).toHaveLength(2);
  expect(accounts).not.toMatch(/private|refresh|cookie|xsrf/);
  expect((await post("/api/communication/gmail/read", { account: "me@example.com", path: "messages?maxResults=1" })).body).toMatchObject({ nextPageToken: "page2" });
  expect((await post("/api/communication/gchat/read", { account: "me@example.com", action: "list" })).body).toMatchObject({ self_id: "123" });
  expect((await post("/api/communication/gchat/write", { account: "me@example.com", action: "send", kind: "space", conversation: "space", text: "private" })).status).toBe(400);
  expect(seen.some(s => s.endsWith("/create_topic"))).toBe(false);
  expect((await post("/api/communication/gchat/write", { account: "me@example.com", action: "send", kind: "space", conversation: "space", text: "private", confirm: true })).body).toMatchObject({ topic: { id: { topic_id: "new" } } });
  const calendar = await (await fetch(base + "/api/auth/stato", { headers: { Authorization: "Bearer key" } })).json();
  expect(calendar).toEqual({ caselle: [] });
  expect(fs.existsSync(path.join(dir, "tokens.json"))).toBe(false);
  expect((await post("/api/auth/rimuovi", { provider: "gchat", account: "me@example.com" })).body).toMatchObject({ ok: true });
  expect((await post("/api/communication/gchat/read", { account: "me@example.com", action: "list" })).status).toBe(404);
});

it("records rejected Chat authentication without deleting the imported session", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oe-google-expired-")); dirs.push(dir);
  const service = new GoogleCommunication(dir, { fetchFn: async input => String(input).includes("RotateCookies") ? new Response(null, { status: 401 }) : new Response('<script>window.WIZ_global_data = {"qwAQke":"AccountsSignInUi"};</script>') });
  service.sessions.metti({ account: "me@example.com", user_agent: "test", salvata: 0, self_id: "123", cookies: [{ name: "SID", value: "expired", domain: ".google.com", path: "/", expires: 0 }] });
  await expect(service.readChat({ account: "me@example.com", action: "list" })).rejects.toThrow(/expired/);
  expect(service.accounts()).toEqual([{ account: "me@example.com", provider: "gchat", self_id: "123", status: "errore", message: "Google Chat session import required" }]);
  expect(service.sessions.sessione("me@example.com")?.cookies.length).toBe(1);
});
