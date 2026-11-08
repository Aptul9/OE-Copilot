import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { GoogleChat, encodeChat, decodeChat } from "../src/gchat.ts";
import { ArchivioGoogleWeb, intestazioneCookie, leggiSessioneImportata, assorbiCookie, type SessioneGoogleWeb } from "../src/google-web.ts";

const NOW = 1_790_000_000;
const session = (): SessioneGoogleWeb => ({ account: "me@example.com", user_agent: "test-browser", salvata: NOW, cookies: [
  { name: "SID", value: "domain", domain: ".google.com", path: "/", expires: NOW + 3600 },
  { name: "OSID", value: "chat-only", domain: "chat.google.com", path: "/", expires: 0 },
] });
const json = (v: unknown, status = 200, headers = {}) => new Response(JSON.stringify(v), { status, headers });
const wire = (type: string, v: Record<string, unknown>) => new Response(Buffer.from(encodeChat(type, v)));
const wiz = (loggedIn = true) => new Response(`<script>window.WIZ_global_data = ${JSON.stringify({ qwAQke: loggedIn ? "ChatUi" : "AccountsSignInUi", SMqcke: "xsrf-test" })};</script>`);
const dirs: string[] = [];
afterEach(() => { for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true }); });

describe("Google session", () => {
  it("scopes cookies by host and path boundary; content hops carry no cookies", () => {
    const cookies = [...session().cookies, { name: "PATH", value: "one", domain: "chat.google.com", path: "/u/0", expires: 0 }];
    expect(intestazioneCookie(cookies, "chat.google.com", "/u/01", NOW)).not.toContain("PATH");
    expect(intestazioneCookie(cookies, "chat.google.com", "/u/0/api", NOW)).toContain("PATH=one");
    expect(intestazioneCookie(cookies, "chat-wsx.usercontent.google.com", "/download", NOW)).toBe("SID=domain");
    expect(intestazioneCookie(cookies, "lh3.googleusercontent.com", "/image", NOW)).toBe("");
    expect(intestazioneCookie(cookies, "google.com.evil.test", "/", NOW)).toBe("");
  });

  it("filters foreign/expired cookies and refuses malformed cookie headers", () => {
    const imported = leggiSessioneImportata({ ...session(), cookies: [...session().cookies,
      { name: "BAD", value: "x\r\nInjected: y", domain: ".google.com" },
      { name: "OLD", value: "x", domain: ".google.com", expires: NOW - 1 },
      { name: "JIRA", value: "x", domain: ".atlassian.net" },
    ] }, NOW);
    expect(imported.cookies.map(c => c.name)).toEqual(["SID", "OSID"]);
    expect(() => leggiSessioneImportata({ cookies: [] }, NOW)).toThrow(/SID/);
  });

  it("absorbs rotation with expiry and removes expired cookies", () => {
    const cookies = session().cookies;
    assorbiCookie(cookies, "accounts.google.com", ["SIDCC=new; Domain=.google.com; Path=/; Max-Age=60", "X=x; Domain=evil.test; Path=/"], NOW);
    expect(cookies.find(c => c.name === "SIDCC")?.expires).toBe(NOW + 60);
    assorbiCookie(cookies, "accounts.google.com", ["SIDCC=; Domain=.google.com; Path=/; Max-Age=0"], NOW);
    expect(cookies.some(c => c.name === "SIDCC")).toBe(false);
    expect(cookies.some(c => c.name === "X")).toBe(false);
  });

  it("persists sessions atomically; restart reads them; failed persistence is visible", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "oe-google-")); dirs.push(dir);
    const file = path.join(dir, "state.json");
    const store = new ArchivioGoogleWeb(file);
    store.metti(session());
    expect(new ArchivioGoogleWeb(file).sessione("ME@EXAMPLE.COM")?.cookies).toEqual(session().cookies);
    const bad = new ArchivioGoogleWeb(path.join(file, "impossible.json"));
    expect(() => bad.metti(session())).toThrow();
    expect(bad.elenco()).toEqual([]);
    expect(fs.readdirSync(dir)).toEqual(["state.json"]);
    fs.writeFileSync(file, "broken json");
    expect(() => new ArchivioGoogleWeb(file).elenco()).toThrow();
  });
});

describe("Chat binary RPC", () => {
  it("matches protobuf field numbers and preserves 64-bit ids", () => {
    expect(Buffer.from(encodeChat("GroupId", { space_id: { space_id: "abc" } })).toString("hex")).toBe("0a050a03616263");
    const input = { client_type: "WEB", client_version: "9007199254740993" };
    expect(decodeChat("RequestHeader", encodeChat("RequestHeader", input))).toEqual(input);
  });

  it("lists and reads without sending read receipts; reports bounded coverage", async () => {
    const calls: string[] = [];
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async (input, init) => {
      const u = new URL(String(input)); calls.push(u.pathname);
      if (u.pathname.endsWith("/mole/world")) return wiz();
      expect(new Headers(init?.headers).get("x-framework-xsrf-token")).toBe("xsrf-test");
      if (u.pathname.endsWith("/get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: "123" } } });
      if (u.pathname.endsWith("/get_members")) return wire("GetMembersResponse", { members: [{ user: { user_id: { id: "123" }, name: "Me", email: "me@example.com" } }] });
      if (u.pathname.endsWith("/paginated_world")) return wire("PaginatedWorldResponse", { world_items: [{ group_id: { space_id: { space_id: "space1" } }, room_name: "Test", sort_timestamp: String(NOW * 1e6), read_state: { unread_message_count: 3 } }] });
      if (u.pathname.endsWith("/list_topics")) {
        const request = decodeChat("ListTopicsRequest", new Uint8Array(init?.body as Uint8Array));
        expect(request.page_size_for_topics).toBe(25);
        return wire("ListTopicsResponse", { topics: [{ id: { topic_id: "t1" }, replies: [{ id: { message_id: "t1" }, creator: { user_id: { id: "123" } }, text_body: "hello", create_time: String(NOW * 1e6) }] }] });
      }
      throw new Error(`unexpected ${u.pathname}`);
    } });
    expect(await client.identity()).toEqual({ account: "me@example.com", self_id: "123" });
    expect((await client.list()).conversations[0]).toMatchObject({ id: "space1", name: "Test", unread: 3, kind: "space" });
    const read = await client.read("space1", "space", 25);
    expect(read).toMatchObject({ self_id: "123", coverage: "recent-topics", messages: [{ id: "t1", thread: "t1", text: "hello", from_id: "123" }] });
    expect(calls.some(p => p.includes("mark_group"))).toBe(false);
  });

  it("renews rejected session once, retries actual identity, persists rotated cookie", async () => {
    let ready = false; let rotated = 0; let saved = 0;
    const s = session();
    const client = new GoogleChat(s, { now: () => NOW * 1000, changed: () => { saved++; }, fetchFn: async input => {
      const u = new URL(String(input));
      if (u.pathname === "/RotateCookies") { ready = true; rotated++; return json({}, 200, { "Set-Cookie": "SIDCC=fresh; Domain=.google.com; Path=/; Max-Age=600" }); }
      if (u.pathname.endsWith("/mole/world")) return wiz(ready);
      if (u.pathname.endsWith("/get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: "123" } } });
      return wire("GetMembersResponse", { members: [{ user: { user_id: { id: "123" }, email: "me@example.com" } }] });
    } });
    await expect(client.identity()).resolves.toMatchObject({ self_id: "123" });
    expect(rotated).toBe(1); expect(saved).toBeGreaterThan(0);
    expect(s.cookies.some(c => c.name === "SIDCC" && c.value === "fresh")).toBe(true);
  });

  it("does not interpret rotation HTTP200 as valid login", async () => {
    let rotate = 0;
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async input => {
      if (String(input).includes("RotateCookies")) { rotate++; return json({}, 200, { "Set-Cookie": "SIDCC=new; Domain=.google.com; Path=/; Max-Age=600" }); }
      return wiz(false);
    } });
    await expect(client.identity()).rejects.toThrow(/session|sign|access/i);
    expect(rotate).toBe(1);
  });

  it("recovers a rejected RPC even while its XSRF page remains accessible", async () => {
    let ready = false; let rotations = 0;
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async input => {
      const u = String(input);
      if (u.includes("mole/world")) return wiz();
      if (u.includes("RotateCookies")) { ready = true; rotations++; return json({}, 200, { "Set-Cookie": "SIDCC=fresh; Domain=.google.com; Path=/; Max-Age=600" }); }
      if (!ready) return json({}, 401);
      if (u.includes("get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: "123" } } });
      return wire("GetMembersResponse", { members: [{ user: { user_id: { id: "123" }, email: "me@example.com" } }] });
    } });
    expect(await client.identity()).toMatchObject({ self_id: "123" });
    expect(rotations).toBe(1);
  });

  it("rejects malformed write flags before making network requests", async () => {
    let count = 0;
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async () => { count++; return new Response(null, { status: 500 }); } });
    for (const change of [{ files: "not-an-array" }, { text: 123 }, { thread: 0 }]) {
      await expect(client.write({ action: "send", conversation: "space1", kind: "space", text: "body", confirm: true, ...change })).rejects.toMatchObject({ status: 400 });
    }
    await expect(client.write({ action: "react", conversation: "space1", kind: "space", message_id: "m", thread: "t", emoji: "a", remove: "true", confirm: true })).rejects.toMatchObject({ status: 400 });
    expect(count).toBe(0);
  });

  it("never replays ambiguous writes or sends without confirmation", async () => {
    let sends = 0;
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async input => {
      if (String(input).includes("mole/world")) return wiz();
      if (String(input).includes("get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: "123" } } });
      sends++; return new Response("secret upstream body", { status: 503 });
    } });
    await expect(client.write({ action: "send", conversation: "space1", kind: "space", text: "private test" })).rejects.toThrow(/confirm/);
    expect(sends).toBe(0);
    await expect(client.write({ action: "send", conversation: "space1", kind: "space", text: "private test", confirm: true })).rejects.toThrow("Google Chat HTTP 503");
    expect(sends).toBe(1);
  });

  it("verifies pinned identity before writes; concurrent requests share renewal", async () => {
    let rotations = 0; let xsrf = 0; let ready = false; let sends = 0;
    const s = session(); s.self_id = "123";
    const client = new GoogleChat(s, { now: () => NOW * 1000, fetchFn: async input => {
      const u = String(input);
      if (u.includes("mole/world")) { xsrf++; return wiz(ready); }
      if (u.includes("RotateCookies")) { rotations++; ready = true; return json({}, 200, { "Set-Cookie": "SIDCC=new; Domain=.google.com; Path=/; Max-Age=300" }); }
      if (u.includes("get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: "other" } } });
      if (u.includes("create_topic")) { sends++; return wire("CreateTopicResponse", {}); }
      return wire("GetMembersResponse", {});
    } });
    const results = await Promise.allSettled([client.identity(), client.identity()]);
    expect(results.every(r => r.status === "rejected")).toBe(true);
    expect(rotations).toBe(1); expect(xsrf).toBe(2);
    await expect(client.write({ action: "send", conversation: "space1", kind: "space", text: "test", confirm: true })).rejects.toThrow(/another account/);
    expect(sends).toBe(0);
  });

  it("reads membership for private-space write checks", async () => {
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async input => {
      if (String(input).includes("mole/world")) return wiz();
      if (String(input).includes("get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: "123" } } });
      return wire("GetGroupResponse", { joined_member_ids: [{ user_id: { id: "123" } }] });
    } });
    expect(await client.details("space1", "space")).toMatchObject({ joined_member_ids: [{ user_id: { id: "123" } }] });
  });

  it.each([false, true])("rechecks pinned account after stale-XSRF renewal before mutations (file=%s)", async withFile => {
    let now = NOW * 1000; let switched = false; let mutations = 0;
    const s = session(); s.self_id = "123"; s.cookies.forEach(c => { c.expires = 0; });
    const client = new GoogleChat(s, { now: () => now, fetchFn: async input => {
      const u = String(input);
      if (u.includes("mole/world")) return wiz(now === NOW * 1000 || switched);
      if (u.includes("RotateCookies")) { switched = true; return json({}, 200, { "Set-Cookie": "SIDCC=fresh; Domain=.google.com; Path=/; Max-Age=600" }); }
      if (u.includes("get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: switched ? "other" : "123" } } });
      if (u.includes("get_members")) return wire("GetMembersResponse", { members: [{ user: { user_id: { id: "123" }, email: "me@example.com" } }] });
      mutations++;
      return u.includes("create_topic") ? wire("CreateTopicResponse", {}) : new Response(null, { status: 500 });
    } });
    await client.identity(); now += 25 * 3600 * 1000;
    await expect(client.write({ action: "send", conversation: "space1", kind: "space", text: "private", confirm: true,
      ...(withFile ? { files: [{ name: "test.txt", data: "aGk=", mime_type: "text/plain" }] } : {}) })).rejects.toMatchObject({ status: 409 });
    expect(mutations).toBe(0);
  });

  it("rejects attachment sign-in redirects instead of returning HTML as a file", async () => {
    let calls = 0;
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async input => {
      calls++;
      return String(input).includes("get_attachment_url") ? new Response(null, { status: 302, headers: { Location: "https://accounts.google.com/ServiceLogin" } }) : new Response("<html>sign in</html>", { headers: { "Content-Type": "text/html" } });
    } });
    await expect(client.attachment("opaque", "image/png")).rejects.toMatchObject({ status: 409 });
    expect(calls).toBe(1);
  });

  it("revalidates identity after a successful XSRF page refresh too", async () => {
    let now = NOW * 1000; let mutations = 0;
    const s = session(); s.self_id = "123"; s.cookies.forEach(c => { c.expires = 0; });
    const client = new GoogleChat(s, { now: () => now, fetchFn: async input => {
      const u = String(input);
      if (u.includes("mole/world")) return wiz();
      if (u.includes("get_self_user_status")) return wire("GetSelfUserStatusResponse", { user_status: { user_id: { id: now === NOW * 1000 ? "123" : "other" } } });
      if (u.includes("get_members")) return wire("GetMembersResponse", { members: [{ user: { user_id: { id: "123" }, email: "me@example.com" } }] });
      mutations++; return wire("CreateTopicResponse", {});
    } });
    await client.identity(); now += 25 * 3600 * 1000;
    await expect(client.write({ action: "send", conversation: "space1", kind: "space", text: "test", confirm: true })).rejects.toMatchObject({ status: 409 });
    expect(mutations).toBe(0);
  });

  it("validates attachment redirect hops; strips cookies outside google.com", async () => {
    const cookies: (string | null)[] = [];
    const client = new GoogleChat(session(), { now: () => NOW * 1000, fetchFn: async (input, init) => {
      cookies.push(new Headers(init?.headers).get("cookie"));
      if (String(input).includes("get_attachment_url")) return new Response(null, { status: 302, headers: { Location: "https://lh3.googleusercontent.com/file" } });
      return new Response("file content");
    } });
    const result = await client.attachment("opaque", "image/png");
    expect(Buffer.from(result.data, "base64").toString()).toBe("file content");
    expect(cookies).toEqual(["OSID=chat-only; SID=domain", null]);
    await expect(client.download("http://127.0.0.1/secret")).rejects.toThrow(/URL|host/);
  });
});
