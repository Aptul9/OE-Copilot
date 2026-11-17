import fs from "node:fs";
import path from "node:path";
import Database from "better-sqlite3";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { GET } from "@/app/api/everflow/saved/route";
import { appDb, claimSlot, migrateAppSchema, setSlotStopped } from "@/lib/appdb";
import { createSlotDb, tempDir } from "./helpers";

const TOKEN = "0123456789abcdef".repeat(4);
const LONG = "x".repeat(2001);
let dataDir: string;
let first: number;
let second: number;
let stopped: number;
let other: number;

function store(slot: number, me: string) {
  const db = createSlotDb(path.join(dataDir, String(slot), "messages.db"));
  const state = db.prepare("INSERT INTO state(k, v) VALUES(?, ?)");
  state.run("me", JSON.stringify({ name: "ROSSI Anna", email: me }));
  state.run("last_scan_ts", "1790431900");
  state.run("state", "ok");
  state.run("health", "{}");
  const chat = db.prepare("INSERT INTO chats(name, preview, pos, tm, unread, mention, muted) VALUES(?, '', ?, '', 0, 0, ?)");
  chat.run("BIANCHI Luca", 0, 0);
  chat.run("Noisy group", 1, 1);
  const msg = db.prepare("INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES(?, ?, ?, ?, ?, ?, '', ?)");
  msg.run("BIANCHI Luca", 0, "1790432000000", "BIANCHI Luca", LONG, 0, JSON.stringify({ mentionsMe: true }));
  msg.run("BIANCHI Luca", 1, "1790431000000", "ROSSI Anna", "Earlier", 1, "");
  msg.run("BIANCHI Luca", 2, "abc", "BIANCHI Luca", "Not saved yet", 0, "");
  db.close();
}

beforeAll(() => {
  dataDir = tempDir();
  process.env.APP_DB = path.join(dataDir, "app.db");
  process.env.ADMIN_EMAIL = "admin@teamsrelay.test";
  migrateAppSchema(appDb());
  appDb().exec('CREATE TABLE IF NOT EXISTS "user"(id TEXT PRIMARY KEY, email TEXT NOT NULL)');
  appDb().prepare('INSERT INTO "user"(id, email) VALUES(?, ?)').run("admin-id", "admin@teamsrelay.test");
  first = claimSlot(appDb(), "admin-id", { perUser: 4 });
  other = claimSlot(appDb(), "someone-else", { perUser: 4 });
  second = claimSlot(appDb(), "admin-id", { perUser: 4 });
  stopped = claimSlot(appDb(), "admin-id", { perUser: 4 });
  setSlotStopped(appDb(), stopped, true);
  store(first, "anna@contoso.example");
  store(second, "anna@fabrikam.example");
  store(stopped, "anna@stopped.example");
  store(other, "luca@other.example");
});

beforeEach(() => {
  process.env.MCP_TOKEN = TOKEN;
});

afterEach(() => {
  delete process.env.MCP_TOKEN;
});

const get = (query: string, headers: Record<string, string> = { Authorization: `Bearer ${TOKEN}` }) =>
  GET(new Request(`http://webapp:8090/api/everflow/saved?${query}`, { headers }), undefined);

type Slot = { slot: number; snapshot?: string; error?: string };

describe("GET /api/everflow/saved", () => {
  it("refuses without the token", async () => {
    const refused: Record<string, string>[] = [{}, { Authorization: "Bearer wrong" }, { Authorization: TOKEN }, { Cookie: "better-auth.session_token=x" }];
    for (const headers of refused) expect((await get("since=0&max_rows=10", headers)).status, JSON.stringify(headers)).toBe(401);
    expect((await get("since=0&max_rows=10", { Authorization: `Bearer ${TOKEN}`, Origin: "https://x" })).status).toBe(403);
    delete process.env.MCP_TOKEN;
    expect((await get("since=0&max_rows=10")).status).toBe(404);
  });

  it("rejects bad parameters", async () => {
    for (const q of ["since=-1&max_rows=10", "since=x&max_rows=10", "since=0&max_rows=0", "since=0&max_rows=100001", "max_rows=10", "since=1.5&max_rows=10", "since=0"])
      expect((await get(q)).status, q).toBe(400);
  });

  it("returns the admin's active slots only, in slot order", async () => {
    const r = await get("since=0&max_rows=10");
    expect(r.status).toBe(200);
    const { slots } = (await r.json()) as { slots: Slot[] };
    expect(slots.map((s) => s.slot)).toEqual([first, second]);
  });

  it("snapshot matches the Everflow store query", async () => {
    const { slots } = (await (await get("since=1790431500000&max_rows=10")).json()) as { slots: Slot[] };
    const saved = JSON.parse(slots[0].snapshot!);
    expect(JSON.parse(saved.state.me).email).toBe("anna@contoso.example");
    expect(saved.state.last_scan_ts).toBe("1790431900");
    expect(saved.state.state).toBe("ok");
    expect(saved.state.health).toBeUndefined();
    expect(saved.muted).toEqual(["Noisy group"]);
    expect(saved.messages).toEqual([
      { chat: "BIANCHI Luca", mid: "1790432000000", author: "BIANCHI Luca", text: "x".repeat(2000), mine: 0, extra: JSON.stringify({ mentionsMe: true }) },
    ]);
    const all = JSON.parse(((await (await get("since=0&max_rows=10")).json()) as { slots: Slot[] }).slots[0].snapshot!);
    expect(all.messages.map((m: { mid: string }) => m.mid)).toEqual(["1790431000000", "1790432000000"]);
  });

  it("max_rows + 1 rows", async () => {
    const { slots } = (await (await get("since=0&max_rows=1")).json()) as { slots: Slot[] };
    expect(JSON.parse(slots[0].snapshot!).messages).toHaveLength(2);
  });

  it("a WAL store with a writer holding an open transaction still reads its committed rows", async () => {
    const writer = new Database(path.join(dataDir, String(first), "messages.db"));
    try {
      writer.exec("BEGIN IMMEDIATE");
      writer.prepare("INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES('BIANCHI Luca', 9, '1790433000000', 'BIANCHI Luca', 'uncommitted', 0, '', '')").run();
      const { slots } = (await (await get("since=0&max_rows=10")).json()) as { slots: Slot[] };
      expect(JSON.parse(slots[0].snapshot!).messages).toHaveLength(2);
    } finally {
      writer.exec("ROLLBACK");
      writer.close();
    }
  });

  it("text that is not UTF-8 is replaced, not the whole slot lost", async () => {
    const db = new Database(path.join(dataDir, String(second), "messages.db"));
    db.prepare("INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES('BIANCHI Luca', 9, '1790433000000', 'BIANCHI Luca', CAST(x'ff41' AS TEXT), 0, '', '')").run();
    db.close();
    const { slots } = (await (await get("since=1790433000000&max_rows=10")).json()) as { slots: Slot[] };
    expect(JSON.parse(slots[1].snapshot!).messages[0].text).toBe("\uFFFDA");
  });

  it("a store locked by its agent is that slot's error within a short wait", async () => {
    const locker = new Database(path.join(dataDir, String(first), "messages.db"));
    try {
      locker.exec("PRAGMA locking_mode=EXCLUSIVE; BEGIN EXCLUSIVE; UPDATE state SET v=v WHERE k='state';");
      const started = Date.now();
      const { slots } = (await (await get("since=0&max_rows=10")).json()) as { slots: Slot[] };
      expect(Date.now() - started).toBeLessThan(4000);
      expect(slots[0]).toEqual({ slot: first, error: "store-unavailable" });
      expect(slots[1].snapshot).toBeTypeOf("string");
    } finally {
      locker.exec("ROLLBACK");
      locker.close();
    }
  });

  it("missing store is that slot's error", async () => {
    fs.rmSync(path.join(dataDir, String(second), "messages.db"));
    const r = await get("since=0&max_rows=10");
    expect(r.status).toBe(200);
    const { slots } = (await r.json()) as { slots: Slot[] };
    expect(slots[0].snapshot).toBeTypeOf("string");
    expect(slots[1]).toEqual({ slot: second, error: "store-unavailable" });
  });
});
