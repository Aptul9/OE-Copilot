import assert from "node:assert/strict";
import { test } from "node:test";
import { resourceId, resource } from "../src/identity.ts";
import { Slack } from "../src/slack.ts";
import { Communication, registerCommunication } from "../src/communication.ts";
import { fake } from "./helpers.ts";
import { McpServer } from "@modelcontextprotocol/server";
test("signed handles reject changed account, type and payload", () => {
  const id = resourceId("slack:T1", "conversation", "C123");
  assert.equal(resource("slack:T1", "conversation", id).native, "C123");
  assert.throws(
    () => resource("slack:T2", "conversation", id),
    /another account/,
  );
  assert.throws(() => resource("slack:T1", "email", id), /another account/);
  assert.throws(
    () => resource("slack:T1", "conversation", "x" + id),
    /Invalid resource/,
  );
});
test("Slack argument values cannot add workspace or shell options", async () => {
  const calls: string[][] = [];
  const s = new Slack(async (args) => {
    calls.push(args);
    return {};
  });
  await s.call("T123", "send", {
    channel: "C123",
    text: "--workspace=T999; echo bad",
    limit: 20,
  });
  assert.deepEqual(calls[0], [
    "messages",
    "send",
    "--workspace=T123",
    "--json",
    "--recipient-id=C123",
    "--message=--workspace=T999; echo bad",
  ]);
  await assert.rejects(s.call("--bad", "list", { limit: 20 }));
  await assert.rejects(
    s.call("T123", "read", { channel: "--workspace=T999", limit: 20 }),
  );
  assert.equal(calls.length, 1);
});
test("exact account IDs reject fragments and missing defaults", async () => {
  const { o } = fake({});
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    undefined,
    undefined,
  );
  await assert.rejects(c.account("msc"), /Unknown exact/);
  await assert.rejects(c.account(""), /required/);
  assert.equal(
    (await c.account("outlook:mario@msc.example")).native,
    "mario@msc.example",
  );
});
test("all data schemas require account_id and old names absent", () => {
  const { o } = fake({});
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    undefined,
    undefined,
  );
  const tools: any[] = [];
  registerCommunication(
    {
      registerTool: (name: any, config: any) => tools.push({ name, config }),
    } as unknown as McpServer,
    c,
  );
  assert.ok(tools.length > 15);
  for (const t of tools) {
    if (t.name === "communication_accounts") continue;
    assert.equal(t.config.inputSchema.safeParse({}).success, false, t.name);
    assert.ok(t.config.inputSchema.shape.account_id, t.name);
  }
  assert.ok(
    !tools.some((t) => t.name === "list_unread" || t.name.startsWith("teams_")),
  );
});
test("cross-account email handles rejected before provider request", async () => {
  const { o, calls } = fake({});
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    undefined,
    undefined,
  );
  const a = await c.account("outlook:mario@msc.example");
  await assert.rejects(
    c.email(a, "read", {
      message_id: resourceId("outlook:other@example.com", "email", "id"),
    }),
    /another account/,
  );
  assert.equal(calls.length, 0);
});

test("Google routing pins account and preserves Gmail pagination", async () => {
  const { o } = fake({});
  const calls: any[] = [];
  const gateway = async (path: string, body?: any) => {
    calls.push({ path, body });
    if (path.endsWith("/accounts"))
      return { accounts: [{ provider: "gmail", account: "g@example.com" }] };
    return { messages: [{ id: "m1", threadId: "t1" }], nextPageToken: "next" };
  };
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    gateway,
    undefined,
  );
  const a = await c.account("gmail:g@example.com");
  const r = await c.email(a, "list", { limit: 5 });
  assert.equal(resource(a.account_id, "email", r.items[0].id).native, "m1");
  assert.equal(resource(a.account_id, "cursor", r.next_cursor).native, "next");
  assert.equal(calls.at(-1).body.account, "g@example.com");
  await assert.rejects(
    c.email(a, "read", {
      message_id: resourceId("gmail:other@example.com", "email", "m1"),
    }),
    /another account/,
  );
});
test("Slack search cannot interpret query as CLI option", async () => {
  const calls: string[][] = [];
  const s = new Slack(async (a) => {
    calls.push(a);
    return {};
  });
  await s.call("T123", "search", { query: "--workspace=EVIL", limit: 5 });
  assert.deepEqual(calls[0], [
    "search",
    "messages",
    "--workspace=T123",
    "--json",
    "--limit=5",
    "--",
    "--workspace=EVIL",
  ]);
});

test("gchat attachment handles bind token and MIME to account", async () => {
  const { o } = fake({});
  const calls: any[] = [];
  const g = async (path: string, body?: any) => {
    calls.push({ path, body });
    if (path.endsWith("/accounts"))
      return { accounts: [{ provider: "gchat", account: "g@example.com" }] };
    if (body.action === "read")
      return {
        messages: [
          {
            id: "m1",
            thread_id: "t1",
            attachments: [{ token: "opaque", mime_type: "text/plain" }],
          },
        ],
      };
    return { data: "aGk=" };
  };
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    g,
    undefined,
  );
  const a = await c.account("gchat:g@example.com");
  const conversation_id = resourceId(a.account_id, "conversation", "space", {
    kind: "space",
  });
  const r = await c.chat(a, "read", { conversation_id, limit: 1 });
  const attachment_id = r.messages[0].attachments[0].id;
  await c.chat(a, "attachment", { attachment_id });
  assert.equal(calls.at(-1).body.token, "opaque");
  assert.equal(calls.at(-1).body.account, "g@example.com");
});

test("Slack rejects editing another user message before write", async () => {
  const calls: string[][] = [];
  const s = new Slack(async (args) => {
    calls.push(args);
    return args[0] === "auth" ? { user_id: "U1" } : { message: { user: "U2" } };
  });
  await assert.rejects(
    s.call("T123", "delete", { channel: "C123", ts: "123.456" }),
    /Only own/,
  );
  assert.equal(calls.length, 2);
  assert.ok(!calls.some((a) => a[0] === "messages"));
});

test("Gmail draft MIME blocks header injection and carries attachments", async () => {
  const { o } = fake({});
  const calls: any[] = [];
  const g = async (path: string, body?: any) => {
    calls.push({ path, body });
    return {};
  };
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    g,
    undefined,
  );
  const a = {
    account_id: "gmail:g@example.com",
    native: "g@example.com",
    provider: "gmail",
    name: "g",
    capabilities: [],
  };
  await c.email(a, "send", {
    to: ["g@example.com"],
    subject: "test",
    text: "body",
    draft: true,
    files: [{ name: "test.txt", mime_type: "text/plain", data: "aGk=" }],
  });
  const request = calls[0].body;
  assert.equal(request.action, "draft");
  const mime = Buffer.from(request.raw, "base64url").toString();
  assert.match(mime, /multipart.mixed/);
  assert.match(mime, /filename="test.txt"/);
  await assert.rejects(
    c.email(a, "send", {
      to: ["bad" + String.fromCharCode(10) + "Bcc: other"],
      subject: "x",
      text: "b",
    }),
    /newline/,
  );
  assert.equal(calls.length, 1);
});

test("one failed provider inventory does not hide working accounts", async () => {
  const { o } = fake({});
  const c = new Communication(
    o,
    {
      tools: [],
      register: () => [],
      refresh: async () => {},
      close: async () => {},
    },
    async () => {
      throw new Error("offline");
    },
    new Slack(async () => ({
      workspaces: [{ profile: "T123", workspace_name: "Test" }],
    })),
  );
  const all = await c.accounts();
  assert.ok(all.some((a) => a.provider === "outlook"));
  assert.ok(all.some((a) => a.provider === "slack"));
  assert.equal(c.errors.google, "Provider inventory unavailable");
});

test("Slack files use generated paths and reject malformed base64 before CLI", async () => {
  const calls: string[][] = [];
  const s = new Slack(async (args) => {
    calls.push(args);
    return { channel_id: "C123" };
  });
  await s.call("T123", "send", {
    channel: "C123",
    text: "file",
    files: [{ name: "../../escape.txt", data: "aGk=" }],
  });
  const file = calls[0]!.find((v) => v.startsWith("--file="));
  assert.ok(file);
  assert.ok(!file.includes("../"));
  assert.ok(calls[0]!.includes("--workspace=T123"));
  await assert.rejects(
    s.call("T123", "send", {
      channel: "C123",
      text: "bad",
      files: [{ name: "x", data: "bad!" }],
    }),
    /base64/,
  );
  assert.equal(calls.length, 1);
});
