import fs from "node:fs";
import path from "node:path";
import type Database from "better-sqlite3";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { GET } from "@/app/api/everflow/attachment/route";
import { appDb, claimSlot, migrateAppSchema, setLight, setSlotStopped } from "@/lib/appdb";
import { createSlotDb, tempDir } from "./helpers";

const TOKEN = "0123456789abcdef".repeat(4);
// smallest PNG: the signature is what the server checks
const PNG = Buffer.concat([Buffer.from("89504e470d0a1a0a", "hex"), Buffer.alloc(16)]);
const MID = "1790432000000";
let dataDir: string;
let first: number;
let stopped: number;
let other: number;
let db: Database.Database;
let timer: ReturnType<typeof setInterval> | undefined;

const now = () => Math.floor(Date.now() / 1000);

beforeAll(() => {
  dataDir = tempDir();
  process.env.APP_DB = path.join(dataDir, "app.db");
  process.env.ADMIN_EMAIL = "admin@teamsrelay.test";
  migrateAppSchema(appDb());
  appDb().exec('CREATE TABLE IF NOT EXISTS "user"(id TEXT PRIMARY KEY, email TEXT NOT NULL)');
  appDb().prepare('INSERT INTO "user"(id, email) VALUES(?, ?)').run("admin-id", "admin@teamsrelay.test");
  first = claimSlot(appDb(), "admin-id", { perUser: 4 });
  other = claimSlot(appDb(), "someone-else", { perUser: 4 });
  stopped = claimSlot(appDb(), "admin-id", { perUser: 4 });
  setSlotStopped(appDb(), stopped, true);
  db = createSlotDb(path.join(dataDir, String(first), "messages.db"));
  for (const slot of [other, stopped]) createSlotDb(path.join(dataDir, String(slot), "messages.db")).close();
  const media = path.join(dataDir, String(first), "media");
  fs.mkdirSync(media, { recursive: true });
  fs.writeFileSync(path.join(media, "00112233445566aa.png"), PNG);
});

afterAll(() => db.close());

beforeEach(() => {
  process.env.MCP_TOKEN = TOKEN;
  setLight(appDb(), first, false);
  db.exec("DELETE FROM chat_messages; DELETE FROM commands; DELETE FROM state");
  db.prepare("INSERT INTO state(k, v) VALUES('health', ?)").run(JSON.stringify({ cdp: "ok", ts: now(), teams: "ok", overall: "green" }));
  const extra = {
    images: [{ f: "00112233445566aa.png" }, { url: "https://eu-api.asm.skype.com/v1/objects/x/views/imgo" }, {}],
    files: [
      { name: "Q3 report.pdf", url: "https://contoso.sharepoint.com/sites/x/Q3 report.pdf" },
      { name: "a.pdf", url: "https://example.com/a.pdf" },
    ],
  };
  db.prepare("INSERT INTO chat_messages(chat, idx, mid, author, text, mine, reacts, extra) VALUES(?, 0, ?, 'BIANCHI Luca', 'see', 0, '', ?)").run("BIANCHI Luca", MID, JSON.stringify(extra));
});

afterEach(() => {
  delete process.env.MCP_TOKEN;
  if (timer) clearInterval(timer);
  timer = undefined;
});

// The agent of the account: runs each pending download, writing `content` as its file
function fakeAgent(content: Buffer | null) {
  timer = setInterval(() => {
    const cmds = db.prepare("SELECT id FROM commands WHERE status='pending' AND type='download'").all() as { id: number }[];
    for (const c of cmds) {
      if (content) {
        fs.mkdirSync(path.join(dataDir, String(first), "files"), { recursive: true });
        fs.writeFileSync(path.join(dataDir, String(first), "files", "0123456789abcdef.pdf"), content);
        db.prepare("INSERT OR REPLACE INTO state(k, v) VALUES(?, ?)").run(`cmd_result:${c.id}`, JSON.stringify({ f: "0123456789abcdef.pdf" }));
      }
      db.prepare("UPDATE commands SET status=? WHERE id=?").run(content ? "done" : "failed", c.id);
    }
  }, 20);
}

const query = (o: Record<string, string | number>) => new URLSearchParams(Object.entries(o).map(([k, v]) => [k, String(v)])).toString();
const get = (q: string, headers: Record<string, string> = { Authorization: `Bearer ${TOKEN}` }) =>
  GET(new Request(`http://webapp:8090/api/everflow/attachment?${q}`, { headers }), undefined);
const ask = (index: number, more: Record<string, string | number> = {}) => get(query({ slot: first, chat: "BIANCHI Luca", mid: MID, index, ...more }));

describe("GET /api/everflow/attachment", () => {
  it("refuses without the token", async () => {
    const q = query({ slot: first, chat: "BIANCHI Luca", mid: MID, index: 1 });
    for (const headers of [{}, { Authorization: "Bearer wrong" }, { Cookie: "better-auth.session_token=x" }] as Record<string, string>[])
      expect((await get(q, headers)).status, JSON.stringify(headers)).toBe(401);
    expect((await get(q, { Authorization: `Bearer ${TOKEN}`, Origin: "https://x" })).status).toBe(403);
    delete process.env.MCP_TOKEN;
    expect((await get(q)).status).toBe(404);
  });

  it("rejects bad parameters", async () => {
    const base = { slot: first, chat: "BIANCHI Luca", mid: MID, index: 1 };
    for (const bad of [{ slot: "x" }, { slot: -1 }, { mid: "abc" }, { mid: "" }, { index: 0 }, { index: 1001 }, { index: "1.5" }, { chat: "" }, { chat: "x".repeat(401) }]) {
      expect((await get(query({ ...base, ...bad }))).status, JSON.stringify(bad)).toBe(400);
    }
    expect((await get(`slot=${first}&mid=${MID}&index=1`)).status).toBe(400);
  });

  it("gives a saved image as its bytes", async () => {
    const r = await ask(1);
    expect(r.status).toBe(200);
    expect(r.headers.get("content-type")).toBe("image/png");
    expect(r.headers.get("content-disposition")).toBe("inline; filename*=UTF-8''00112233445566aa.png");
    expect(r.headers.get("cache-control")).toBe("private, max-age=86400");
    expect(Buffer.from(await r.arrayBuffer())).toEqual(PNG);
  });

  it("gives a SharePoint file through the download command", async () => {
    fakeAgent(Buffer.from("%PDF-1.7"));
    const r = await ask(4);
    expect(r.status).toBe(200);
    expect(r.headers.get("content-type")).toBe("application/pdf");
    expect(r.headers.get("content-disposition")).toBe("inline; filename*=UTF-8''Q3%20report.pdf");
    expect(await r.text()).toBe("%PDF-1.7");
    const queued = db.prepare("SELECT type, arg1, arg2 FROM commands").all();
    expect(queued).toEqual([{ type: "download", arg1: "https://contoso.sharepoint.com/sites/x/Q3 report.pdf", arg2: JSON.stringify({ name: "Q3 report.pdf" }) }]);
  });

  it("over 20 MB is 413", async () => {
    fakeAgent(Buffer.alloc(20e6 + 1));
    const r = await ask(4);
    expect(r.status).toBe(413);
    expect((await r.json()).error).toMatch(/too large/);
  });

  it("an attachment that cannot be given now is 409 with the reason", async () => {
    for (const index of [2, 3, 5]) {
      const r = await ask(index);
      expect(r.status, String(index)).toBe(409);
      expect((await r.json()).error).toBeTypeOf("string");
    }
    fakeAgent(null);
    expect(await (await ask(4)).json()).toEqual({ error: "Teams did not give the file" });
    setLight(appDb(), first, true);
    const light = await ask(4);
    expect(light.status).toBe(409);
    expect((await light.json()).error).toMatch(/runs without a browser/);
  });

  it("no such account, message or index is 404", async () => {
    for (const more of [{ slot: other }, { slot: stopped }, { slot: 999 }, { mid: "1" }, { chat: "Nobody" }, { index: 6 }] as Record<string, string | number>[]) {
      const r = await ask(1, more);
      expect(r.status, JSON.stringify(more)).toBe(404);
      expect((await r.json()).error).toBeTypeOf("string");
    }
  });
});
