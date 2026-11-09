import assert from "node:assert/strict";
import { test } from "node:test";
import type { McpServer } from "@modelcontextprotocol/server";
import { Communication, registerCommunication } from "../src/communication.ts";
import { resource, resourceId } from "../src/identity.ts";
import { replaceDraftBody } from "../src/send.ts";
import { ServiceOutlook } from "../src/service.ts";
import { fake } from "./helpers.ts";

const teams = { tools: [], register: () => [], refresh: async () => {}, close: async () => {} };
const OUT = "outlook:mario@msc.example";
const GM = "gmail:me@gmail.example";
const outlookAccount = { account_id: OUT, provider: "outlook", native: "mario@msc.example", name: "m", capabilities: [] };
const gmailAccount = { account_id: GM, provider: "gmail", native: "me@gmail.example", name: "g", capabilities: [] };
const handle = (account: string, kind: string, native: string) => resourceId(account, kind, native);
const rcpt = (...a: string[]) => a.map((Address) => ({ EmailAddress: { Address } }));
const file = { name: "note é.txt", mime_type: "text/plain", data: Buffer.from("hi").toString("base64") };

function outlook(routes: Parameters<typeof fake>[0]) {
  const f = fake(routes);
  return { c: new Communication(f.o, teams, undefined, undefined), calls: f.calls };
}

test("email tools: every new tool registered with account_id, recipient limits as to", () => {
  const tools: any[] = [];
  registerCommunication({ registerTool: (name: any, config: any) => tools.push({ name, config }) } as unknown as McpServer, outlook({}).c);
  const byName = Object.fromEntries(tools.map((t) => [t.name, t.config.inputSchema]));
  for (const n of ["email_send", "email_reply", "email_reply_all", "email_forward", "email_draft_update", "email_draft_send", "email_draft_delete"]) assert.ok(byName[n], n);
  const many = Array.from({ length: 101 }, (_, i) => `a${i}@x.it`);
  const base = { account_id: OUT, to: ["a@x.it"], subject: "s", text: "t" };
  assert.equal(byName.email_send.safeParse({ ...base, cc: ["b@x.it"], bcc: ["c@x.it"], reply_to: ["d@x.it"] }).success, true);
  assert.equal(byName.email_send.safeParse({ ...base, cc: many }).success, false);
  assert.equal(byName.email_send.safeParse({ ...base, bcc: many }).success, false);
  assert.equal(byName.email_reply.safeParse({ account_id: OUT, message_id: "m", text: "t" }).success, true);
  assert.equal(byName.email_forward.safeParse({ account_id: OUT, message_id: "m" }).success, false);
  assert.equal(byName.email_draft_send.safeParse({ account_id: OUT, draft_id: "d" }).success, true);
});

test("Outlook send: cc, bcc and reply_to on sendmail; plain text as Text, HTML as HTML", async () => {
  const { c, calls } = outlook({ "POST me/sendmail": () => undefined });
  const r = await c.email(outlookAccount, "send", { to: ["a@x.it"], cc: ["b@x.it"], bcc: ["c@x.it"], reply_to: ["r@x.it"], subject: "s", text: "plain" });
  const m = calls[0]!.body.Message;
  assert.deepEqual([m.CcRecipients, m.BccRecipients, m.ReplyTo, m.Body.ContentType], [rcpt("b@x.it"), rcpt("c@x.it"), rcpt("r@x.it"), "Text"]);
  assert.deepEqual([r.status, r.cc, r.bcc, r.draft_id], ["sent", ["b@x.it"], ["c@x.it"], null]);
  await c.email(outlookAccount, "send", { to: ["a@x.it"], subject: "s", text: "<p>html</p>" });
  assert.equal(calls[1]!.body.Message.Body.ContentType, "HTML");
});

test("Outlook reply and reply-all add to/cc on top of the proposed recipients and keep the thread", async () => {
  const draft = { Id: "d1", Subject: "RE: x", ToRecipients: rcpt("boss@y.it"), CcRecipients: rcpt("c@y.it") };
  const { c, calls } = outlook({
    "POST me/messages/m1/createreplyall": () => draft,
    "POST me/messages/m1/createreply": () => ({ ...draft, CcRecipients: [] }),
    "PATCH me/messages/d1": () => ({}),
    "POST me/messages/d1/attachments": () => ({}),
    "POST me/messages/d1/send": () => undefined,
  });
  const message_id = handle(OUT, "email", "m1");
  const all = await c.email(outlookAccount, "reply_all", { message_id, text: "ok", to: ["extra@y.it"], cc: ["new@y.it"] });
  assert.deepEqual([all.status, all.to, all.cc], ["sent", ["boss@y.it", "extra@y.it"], ["c@y.it", "new@y.it"]]);
  assert.deepEqual(calls.map((v) => `${v.method} ${v.url}`), ["POST me/messages/m1/createreplyall", "PATCH me/messages/d1", "POST me/messages/d1/send"]);
  calls.length = 0;
  const one = await c.email(outlookAccount, "reply", { message_id, text: "ok", cc: ["new@y.it", "BOSS@y.it"], files: [file], draft: true });
  assert.deepEqual([one.status, one.to, one.cc], ["draft", ["boss@y.it"], ["new@y.it"]]);
  assert.equal(resource(OUT, "email_draft", one.draft_id).native, "d1");
  assert.equal(resource(OUT, "email", one.message_id).native, "d1");
  assert.deepEqual(calls.map((v) => `${v.method} ${v.url}`), ["POST me/messages/m1/createreply", "PATCH me/messages/d1", "POST me/messages/d1/attachments"]);
  assert.equal(calls[2]!.body.Name, "note é.txt");
});

test("Outlook old email_send message_id stays a reply whose recipients replace the proposed ones", async () => {
  const { c, calls } = outlook({
    "POST me/messages/m1/createreply": () => ({ Id: "d1", ToRecipients: rcpt("boss@y.it") }),
    "PATCH me/messages/d1": () => ({}),
    "POST me/messages/d1/send": () => undefined,
  });
  const r = await c.email(outlookAccount, "send", { message_id: handle(OUT, "email", "m1"), to: ["only@y.it"], subject: "", text: "ok" });
  assert.deepEqual(r.to, ["only@y.it"]);
  assert.deepEqual(calls[1]!.body.ToRecipients, rcpt("only@y.it"));
});

test("Outlook forward: createforward keeps original attachments server side, adds files, cc as given", async () => {
  const { c, calls } = outlook({
    "POST me/messages/m1/createforward": () => ({ Id: "f1", CcRecipients: rcpt("old@y.it") }),
    "PATCH me/messages/f1": () => ({}),
    "POST me/messages/f1/attachments": () => ({}),
    "POST me/messages/f1/send": () => undefined,
  });
  const r = await c.email(outlookAccount, "forward", { message_id: handle(OUT, "email", "m1"), to: ["t@y.it"], cc: ["c@y.it"], text: "fyi", files: [file] });
  assert.deepEqual([r.status, r.to, r.cc], ["sent", ["t@y.it"], ["c@y.it"]]);
  assert.match(calls[0]!.body.Comment, /fyi/);
  assert.deepEqual(calls.map((v) => `${v.method} ${v.url}`), ["POST me/messages/m1/createforward", "PATCH me/messages/f1", "POST me/messages/f1/attachments", "POST me/messages/f1/send"]);
});

test("Outlook drafts: update keeps the quoted original, send and delete only real drafts", async () => {
  const quoted = '<html><head><style>p{}</style></head><body><div>old text</div><hr style="x"><div id="divRplyFwdMsg">From: boss</div><div>original</div></body></html>';
  let isDraft = true;
  const { c, calls } = outlook({
    "GET me/messages/d1?": () => ({ IsDraft: isDraft, Subject: "RE: x", Body: { ContentType: "HTML", Content: quoted }, ToRecipients: rcpt("boss@y.it") }),
    "PATCH me/messages/d1": (v) => v.body,
    "POST me/messages/d1/send": () => undefined,
    "DELETE me/messages/d1": () => undefined,
  });
  const draft_id = handle(OUT, "email_draft", "d1");
  const u = await c.email(outlookAccount, "draft_update", { draft_id, text: "new text", cc: ["c@y.it"] });
  const patch = calls[1]!.body;
  assert.match(patch.Body.Content, /^<html><head><style>p\{\}<\/style><\/head><body><div style="[^"]*">new text<\/div><hr style="x"><div id="divRplyFwdMsg">From: boss<\/div><div>original<\/div><\/body><\/html>$/);
  assert.deepEqual([patch.CcRecipients, patch.ToRecipients], [rcpt("c@y.it"), undefined]);
  assert.deepEqual([u.status, u.to, u.cc], ["draft", ["boss@y.it"], ["c@y.it"]]);
  const s = await c.email(outlookAccount, "draft_send", { draft_id });
  assert.deepEqual([s.status, s.to, calls.at(-1)!.url], ["sent", ["boss@y.it"], "me/messages/d1/send"]);
  assert.deepEqual(await c.email(outlookAccount, "draft_delete", { draft_id }), { status: "deleted" });
  isDraft = false;
  const before = calls.length;
  await assert.rejects(c.email(outlookAccount, "draft_send", { draft_id }), /not a draft/);
  assert.equal(calls.length, before + 1);
  await assert.rejects(c.email(outlookAccount, "draft_send", { draft_id: handle(OUT, "email", "d1") }), /another account or resource type/);
});

test("Outlook draft body without a quote is replaced whole, type by content", () => {
  assert.deepEqual(replaceDraftBody({ ContentType: "Text", Content: "old" }, "new"), { ContentType: "Text", Content: "new" });
  assert.deepEqual(replaceDraftBody({ ContentType: "HTML", Content: "<p>old</p>" }, "<b>new</b>"), { ContentType: "HTML", Content: "<b>new</b>" });
  assert.match(replaceDraftBody({ ContentType: "HTML", Content: '<div id="appendonsend"></div><p>q</p>' }, "a & b").Content, /^<div style="[^"]*">a &amp; b<\/div><div id="appendonsend"><\/div><p>q<\/p>$/);
  // Shape read back from a real Outlook reply draft
  const stored = '<html><body><div><div style="font-size:11pt">v1</div><hr tabindex="-1" style="display:inline-block; width:98%"><div id="x_divRplyFwdMsg" dir="ltr">From: x</div><div>orig</div></div></body></html>';
  assert.match(replaceDraftBody({ ContentType: "HTML", Content: stored }, "v2").Content, /^<html><body><div style="[^"]*">v2<\/div><hr tabindex="-1"[^>]*><div id="x_divRplyFwdMsg" dir="ltr">From: x<\/div><div>orig<\/div><\/div><\/body><\/html>$/);
});

test("Outlook manage maps read and unread to the REST actions", async () => {
  const { c, calls } = outlook({ "PATCH me/messages/m1": () => ({}) });
  const r = await c.email(outlookAccount, "manage", { message_id: handle(OUT, "email", "m1"), action: "read" });
  assert.deepEqual([r.items[0].status, calls[0]!.body], ["marked_read", { IsRead: true }]);
});

test("Outlook service mode refuses every compose tool explicitly", async () => {
  const c = new Communication(new ServiceOutlook(async () => ({ accounts: [] })), teams, undefined, undefined);
  for (const op of ["reply", "reply_all", "forward", "draft_update", "draft_send", "draft_delete"])
    await assert.rejects(c.email(outlookAccount, op, { message_id: handle(OUT, "email", "m1"), draft_id: handle(OUT, "email_draft", "d1") }), (e: any) => e.code === "UNSUPPORTED_CAPABILITY");
});

// Fake agent: Gmail reads by path, writes recorded with their decoded MIME.
function gmail(messages: Record<string, any>, drafts: Record<string, any> = {}) {
  const writes: any[] = [];
  const reads: string[] = [];
  const g = async (path: string, body: any) => {
    if (path.endsWith("/read")) {
      reads.push(body.path);
      const att = /^messages\/([^/]+)\/attachments\/(.+)$/.exec(body.path);
      if (att) return { data: Buffer.from("orig-" + att[2]).toString("base64url") };
      const m = /^(messages|drafts)\/([^?]+)/.exec(body.path)!;
      return (m[1] === "messages" ? messages : drafts)[decodeURIComponent(m[2]!)];
    }
    writes.push({ ...body, mime: body.raw ? Buffer.from(body.raw, "base64url").toString() : undefined });
    if (body.action === "draft" || body.action === "update_draft") return { id: body.draft_id || "dr1", message: { id: "dm1", threadId: body.thread_id || "nt" } };
    return body.action === "delete_draft" ? {} : { id: "sm1", threadId: body.thread_id || "nt" };
  };
  return { c: new Communication(fake({}).o, teams, g, undefined), writes, reads };
}
const header = (mime: string, name: string) => new RegExp("^" + name + ": (.*)$", "mi").exec(mime)?.[1];
const part = (mime: string, type: string) => {
  const m = new RegExp("Content-Type: " + type + "; charset=UTF-8\\r\\nContent-Transfer-Encoding: base64\\r\\n\\r\\n([A-Za-z0-9+/=\\r\\n]+)").exec(mime);
  return m ? Buffer.from(m[1]!.replace(/\r\n/g, ""), "base64").toString() : undefined;
};
const b64u = (s: string) => Buffer.from(s).toString("base64url");
const original = {
  id: "o1",
  threadId: "t1",
  payload: {
    headers: [
      { name: "From", value: '"Boss, Big" <boss@y.it>' },
      { name: "To", value: "me@gmail.example, Peer <peer@y.it>" },
      { name: "Cc", value: "Me Again <ME@gmail.example>, c@y.it, undisclosed-recipients:;" },
      { name: "Subject", value: "Plan" },
      { name: "Date", value: "Mon, 9 Nov 2026 10:00:00 +0100" },
      { name: "Message-ID", value: "<orig@y.it>" },
      { name: "References", value: "<root@y.it>" },
    ],
    mimeType: "multipart/mixed",
    parts: [
      { mimeType: "text/plain", body: { data: b64u("line one\nline two") } },
      { mimeType: "application/pdf", filename: "spec.pdf", body: { attachmentId: "A1", size: 6 } },
    ],
  },
};

test("Gmail send: cc, bcc, reply_to headers; HTML body by content; non-ASCII subject encoded", async () => {
  const { c, writes } = gmail({});
  const r = await c.email(gmailAccount, "send", { to: ["a@x.it"], cc: ["b@x.it"], bcc: ["c@x.it"], reply_to: ["r@x.it"], subject: "Perché", text: "<p>ciao</p>", files: [file] });
  const mime = writes[0].mime;
  assert.deepEqual([header(mime, "To"), header(mime, "Cc"), header(mime, "Bcc"), header(mime, "Reply-To")], ["a@x.it", "b@x.it", "c@x.it", "r@x.it"]);
  assert.equal(header(mime, "Subject"), "=?UTF-8?B?" + Buffer.from("Perché").toString("base64") + "?=");
  assert.equal(part(mime, "text/html"), "<p>ciao</p>");
  assert.match(mime, /filename\*=UTF-8''note%20%C3%A9\.txt/);
  assert.deepEqual([writes[0].action, r.status, r.cc, r.bcc, resource(GM, "email", r.message_id).native, resource(GM, "email_thread", r.thread_id).native], ["send", "sent", ["b@x.it"], ["c@x.it"], "sm1", "nt"]);
  await c.email(gmailAccount, "send", { to: ["a@x.it"], subject: "s", text: "a < b" });
  assert.equal(part(writes[1].mime, "text/plain"), "a < b");
  await assert.rejects(c.email(gmailAccount, "send", { to: ["a@x.it"], cc: ["x\r\nBcc: evil@x.it"], subject: "s", text: "t" }), /newline|Invalid email/);
  assert.equal(writes.length, 2);
});

test("Gmail reply keeps thread headers, answers the sender, quotes the original", async () => {
  const { c, writes } = gmail({ o1: original });
  const message_id = handle(GM, "email", "o1");
  const r = await c.email(gmailAccount, "reply", { message_id, text: "ok", cc: ["extra@y.it"] });
  const mime = writes[0].mime;
  assert.deepEqual([writes[0].thread_id, header(mime, "In-Reply-To"), header(mime, "References"), header(mime, "Subject")], ["t1", "<orig@y.it>", "<root@y.it> <orig@y.it>", "Re: Plan"]);
  assert.deepEqual([r.to, r.cc], [["boss@y.it"], ["extra@y.it"]]);
  assert.match(part(mime, "text/plain")!, /^ok\r\n\r\nOn Mon, 9 Nov 2026 10:00:00 \+0100 "Boss, Big" <boss@y.it> wrote:\r\n> line one\r\n> line two$/);
});

test("Gmail reply-all: sender plus original To and Cc without own address, extras on top", async () => {
  const { c } = gmail({ o1: original });
  const r = await c.email(gmailAccount, "reply_all", { message_id: handle(GM, "email", "o1"), text: "ok", to: ["t2@y.it"], cc: ["c2@y.it", "peer@y.it"] });
  assert.deepEqual([r.to, r.cc], [["boss@y.it", "peer@y.it", "t2@y.it"], ["c@y.it", "c2@y.it"]]);
});

test("Gmail reply to own sent mail goes to its recipients", async () => {
  const own = { ...original, payload: { ...original.payload, headers: [{ name: "From", value: "me@gmail.example" }, { name: "To", value: "other@y.it" }, { name: "Message-ID", value: "<s@g>" }] } };
  const { c } = gmail({ o1: own });
  assert.deepEqual((await c.email(gmailAccount, "reply", { message_id: handle(GM, "email", "o1"), text: "x" })).to, ["other@y.it"]);
  own.payload.headers.push({ name: "Reply-To", value: "desk@y.it" });
  const r = gmail({ o1: own }).c;
  for (const op of ["reply", "reply_all"]) assert.deepEqual((await r.email(gmailAccount, op, { message_id: handle(GM, "email", "o1"), text: "x" })).to[0], "desk@y.it", op);
});

test("Gmail old email_send message_id: recipients and subject as given, thread kept, no quote", async () => {
  const { c, writes } = gmail({ o1: original });
  await c.email(gmailAccount, "send", { message_id: handle(GM, "email", "o1"), to: ["only@y.it"], subject: "Re: Plan", text: "short" });
  const mime = writes[0].mime;
  assert.deepEqual([header(mime, "To"), header(mime, "In-Reply-To"), writes[0].thread_id, part(mime, "text/plain")], ["only@y.it", "<orig@y.it>", "t1", "short"]);
});

test("Gmail forward re-attaches original files, adds new ones, quotes the original header block", async () => {
  const { c, writes, reads } = gmail({ o1: original });
  const r = await c.email(gmailAccount, "forward", { message_id: handle(GM, "email", "o1"), to: ["t@y.it"], cc: ["c@y.it"], text: "fyi", files: [file] });
  const mime = writes[0].mime;
  assert.ok(reads.includes("messages/o1/attachments/A1"));
  assert.equal(header(mime, "Subject"), "Fwd: Plan");
  assert.match(mime, /filename="spec.pdf"[\s\S]*filename="=\?UTF-8\?B\?bm90ZSDDqS50eHQ=\?="/);
  assert.ok(mime.includes(Buffer.from("orig-A1").toString("base64")));
  assert.match(part(mime, "text/plain")!, /^fyi\r\n\r\n---------- Forwarded message ---------\r\nFrom: "Boss, Big" <boss@y.it>[\s\S]*Subject: Plan[\s\S]*\r\n\r\nline one\r\nline two$/);
  assert.deepEqual([r.to, r.cc, r.status], [["t@y.it"], ["c@y.it"], "sent"]);
});

test("Gmail drafts: created by any compose tool, updated with quote and attachments kept, sent, deleted", async () => {
  const draftMessage = {
    id: "dm1",
    threadId: "t1",
    payload: {
      headers: [
        { name: "To", value: "boss@y.it" },
        { name: "Bcc", value: "hidden@y.it" },
        { name: "Subject", value: "Re: Plan" },
        { name: "In-Reply-To", value: "<orig@y.it>" },
        { name: "References", value: "<root@y.it> <orig@y.it>" },
      ],
      mimeType: "multipart/mixed",
      parts: [
        { mimeType: "text/plain", body: { data: b64u("first try\r\n\r\nOn Mon boss wrote:\r\n> line one") } },
        { mimeType: "application/pdf", filename: "spec.pdf", body: { attachmentId: "A9" } },
      ],
    },
  };
  const { c, writes } = gmail({ o1: original }, { dr1: { id: "dr1", message: draftMessage } });
  const created = await c.email(gmailAccount, "reply", { message_id: handle(GM, "email", "o1"), text: "draft", draft: true });
  assert.deepEqual([writes[0].action, created.status, resource(GM, "email_draft", created.draft_id).native], ["draft", "draft", "dr1"]);
  const draft_id = created.draft_id;
  const u = await c.email(gmailAccount, "draft_update", { draft_id, text: "second try", cc: ["c@y.it"] });
  const w = writes[1];
  assert.deepEqual([w.action, w.draft_id, w.thread_id], ["update_draft", "dr1", "t1"]);
  assert.equal(part(w.mime, "text/plain"), "second try\r\n\r\nOn Mon boss wrote:\r\n> line one");
  assert.deepEqual([header(w.mime, "To"), header(w.mime, "Cc"), header(w.mime, "Bcc"), header(w.mime, "In-Reply-To")], ["boss@y.it", "c@y.it", "hidden@y.it", "<orig@y.it>"]);
  assert.ok(w.mime.includes(Buffer.from("orig-A9").toString("base64")));
  assert.deepEqual([u.status, u.cc], ["draft", ["c@y.it"]]);
  const s = await c.email(gmailAccount, "draft_send", { draft_id });
  assert.deepEqual([writes[2].action, writes[2].draft_id, s.status, s.to, s.bcc], ["send_draft", "dr1", "sent", ["boss@y.it"], ["hidden@y.it"]]);
  assert.deepEqual(await c.email(gmailAccount, "draft_delete", { draft_id }), { status: "deleted" });
  assert.equal(writes[3].action, "delete_draft");
  await assert.rejects(c.email(gmailAccount, "draft_send", { draft_id: handle("gmail:other@gmail.example", "email_draft", "dr1") }), /another account/);
});
