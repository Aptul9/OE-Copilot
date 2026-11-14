// Gmail compose over the OE Copilot agent: MIME for new mail, replies, forwards and drafts. The agent holds the grant.
import { CommunicationError } from "./identity.ts";
import { htmlEscape, htmlToMarkdown, looksLikeHtml } from "./html.ts";
import { parseRecipients, type Recipients } from "./send.ts";
import type { GatewayCall } from "./service.ts";

export type GmailFile = { name: string; mime_type?: string; data: string };
export type GmailResult = {
  status: "sent" | "draft" | "deleted";
  message_id?: string;
  thread_id?: string;
  draft_id?: string | null;
  subject?: string;
  to?: string[];
  cc?: string[];
  bcc?: string[];
};
export type Compose = {
  to?: Recipients;
  cc?: Recipients;
  bcc?: Recipients;
  reply_to?: Recipients;
  subject?: string;
  text?: string;
  files?: GmailFile[];
  draft?: boolean;
};

const NL = "\r\n";
const FILES_MAX = 5_000_000;
// The agent refuses a raw message longer than this (base64url characters).
const RAW_MAX = 8 * 1024 * 1024;
const QUOTE_TEXT = /\r?\n\r?\n(?=On [^\r\n]* wrote:\r?\n|---------- Forwarded message ---------)/;
const QUOTE_HTML = /<div class="gmail_quote">/;

const invalid = (message: string): never => {
  throw new CommunicationError("INVALID_INPUT", message);
};
const safe = (v: string) => (/[\r\n]/.test(v) ? invalid("Header newline refused") : v);
const wrap = (b64: string) => b64.replace(/.{1,76}/g, (l) => l + NL).trimEnd();

/** RFC 2047 encoded words for non-ASCII header text, folded so no line passes 998 characters. */
function word(v: string): string {
  safe(v);
  if (/^[\x20-\x7e]*$/.test(v)) return v;
  const chunks: string[] = [];
  let cur = "";
  for (const ch of v) {
    if (Buffer.byteLength(cur + ch) > 45) {
      chunks.push(cur);
      cur = "";
    }
    cur += ch;
  }
  if (cur) chunks.push(cur);
  return chunks.map((c) => "=?UTF-8?B?" + Buffer.from(c).toString("base64") + "?=").join(NL + " ");
}

const ADDRESS = /^[^\s@<>",;]+@[^\s@<>",;]+$/;

function addresses(v: Recipients): string[] {
  return parseRecipients(v).map((r) => {
    const a = safe(r.EmailAddress.Address);
    if (!ADDRESS.test(a)) invalid("Invalid email address");
    const n = r.EmailAddress.Name;
    return n ? word(/^[\x20-\x7e]*$/.test(n) ? '"' + n.replace(/["\\]/g, "") + '"' : n) + " <" + a + ">" : a;
  });
}

/** Address list from a received header: commas inside quotes or angle brackets do not split; groups and junk are dropped. */
function fromHeader(v: string | undefined): string[] {
  const items: string[] = [];
  let cur = "";
  let quoted = false;
  let angle = false;
  for (const ch of v || "") {
    if (ch === '"') quoted = !quoted;
    else if (!quoted && ch === "<") angle = true;
    else if (!quoted && ch === ">") angle = false;
    if (!quoted && !angle && (ch === "," || ch === ";")) {
      items.push(cur);
      cur = "";
    } else cur += ch;
  }
  items.push(cur);
  return items
    .map((s) => s.replace(/^[^"<]*:/, "").trim())
    .filter((s) => ADDRESS.test(/<([^>]+)>/.exec(s)?.[1]?.trim() ?? s))
    .flatMap((s) => addresses([s]));
}
const bare = (list: string[]) => list.map((v) => /<([^>]+)>/.exec(v)?.[1] ?? v);
const lower = (v: string) => (/<([^>]+)>/.exec(v)?.[1] ?? v).trim().toLowerCase();

function merge(base: string[], extra: string[], without: Set<string> = new Set()): string[] {
  const seen = new Set(without);
  const out: string[] = [];
  for (const v of [...base, ...extra]) {
    const k = lower(v);
    if (k && !seen.has(k)) {
      seen.add(k);
      out.push(v);
    }
  }
  return out;
}

function checkFiles(files: GmailFile[] = []): GmailFile[] {
  let total = 0;
  for (const f of files) {
    if (!/^[A-Za-z0-9+/]*={0,2}$/.test(f.data) || f.data.length % 4) invalid("Invalid base64 file");
    total += Buffer.from(f.data, "base64").length;
    if (total > FILES_MAX) throw new CommunicationError("LIMIT_EXCEEDED", "Files exceed 5 MB");
    safe(f.name);
    if (f.mime_type && !/^[A-Za-z0-9.+-]+[/][A-Za-z0-9.+-]+$/.test(f.mime_type)) invalid("Invalid MIME type");
  }
  return files;
}

type Message = {
  to: string[];
  cc: string[];
  bcc: string[];
  reply_to: string[];
  subject: string;
  in_reply_to?: string;
  references?: string;
  body: string;
  html: boolean;
  files: GmailFile[];
};

export function mime(m: Message): string {
  const head: string[] = [];
  if (m.to.length) head.push("To: " + m.to.join(", "));
  if (m.cc.length) head.push("Cc: " + m.cc.join(", "));
  if (m.bcc.length) head.push("Bcc: " + m.bcc.join(", "));
  if (m.reply_to.length) head.push("Reply-To: " + m.reply_to.join(", "));
  head.push("Subject: " + word(m.subject));
  if (m.in_reply_to) head.push("In-Reply-To: " + safe(m.in_reply_to));
  if (m.references) head.push("References: " + safe(m.references));
  head.push("MIME-Version: 1.0");
  const text =
    "Content-Type: text/" +
    (m.html ? "html" : "plain") +
    "; charset=UTF-8" +
    NL +
    "Content-Transfer-Encoding: base64" +
    NL +
    NL +
    wrap(Buffer.from(m.body.replace(/\r?\n/g, NL)).toString("base64"));
  let body = text;
  if (m.files.length) {
    const boundary = "oe_mcp_" + crypto.randomUUID().replace(/-/g, "");
    const parts = [text];
    for (const f of m.files) {
      const name = safe(f.name).replace(/^.*[\\/]/, "") || "attachment";
      // Gmail shows the plain filename parameter and ignores filename*: non-ASCII names go there as one encoded word.
      const ascii = /^[\x20-\x7e]*$/.test(name) ? name.replace(/["\\]/g, "_") : "=?UTF-8?B?" + Buffer.from(name).toString("base64") + "?=";
      const type = f.mime_type || "application/octet-stream";
      parts.push(
        "Content-Type: " + type + '; name="' + ascii + '"' + NL +
          'Content-Disposition: attachment; filename="' + ascii + "\"; filename*=UTF-8''" + encodeURIComponent(name).replace(/['()*]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase()) + NL +
          "Content-Transfer-Encoding: base64" + NL + NL +
          wrap(f.data),
      );
    }
    body =
      "Content-Type: multipart/mixed; boundary=" + boundary + NL + NL +
      "--" + boundary + NL + parts.join(NL + "--" + boundary + NL) + NL + "--" + boundary + "--";
  }
  const raw = Buffer.from(head.join(NL) + NL + body).toString("base64url");
  if (raw.length > RAW_MAX) throw new CommunicationError("LIMIT_EXCEEDED", "Message with attachments exceeds the Gmail size limit");
  return raw;
}

type Parsed = { headers: Record<string, string>; text?: string; html?: string; files: GmailFile[] };
const decode = (data?: string) => (data ? Buffer.from(data, "base64url").toString("utf8") : "");
const b64 = (data: string) => Buffer.from(data, "base64url").toString("base64");

export class Gmail {
  readonly call: GatewayCall;
  readonly account: string;
  constructor(call: GatewayCall, account: string) {
    this.call = call;
    this.account = account.toLowerCase();
  }
  read(path: string) {
    return this.call("/api/communication/gmail/read", { account: this.account, path });
  }
  write(body: Record<string, unknown>) {
    return this.call("/api/communication/gmail/write", { account: this.account, confirm: true, ...body });
  }

  /** Headers, first text/plain and text/html bodies, and every attachment with its bytes. */
  async parse(message: any, withFiles: boolean): Promise<Parsed> {
    const headers: Record<string, string> = {};
    for (const h of message.payload?.headers || []) headers[String(h.name).toLowerCase()] ??= String(h.value);
    const out: Parsed = { headers, files: [] };
    const walk = async (part: any) => {
      if (part.filename || part.body?.attachmentId) {
        if (withFiles) {
          const data = part.body?.attachmentId
            ? (await this.read("messages/" + encodeURIComponent(message.id) + "/attachments/" + encodeURIComponent(part.body.attachmentId))).data
            : part.body?.data;
          out.files.push({ name: part.filename || "attachment", mime_type: part.mimeType, data: b64(String(data || "")) });
        }
      } else if (part.mimeType === "text/plain" && out.text === undefined) out.text = decode(part.body?.data);
      else if (part.mimeType === "text/html" && out.html === undefined) out.html = decode(part.body?.data);
      for (const child of part.parts || []) await walk(child);
    };
    await walk(message.payload || {});
    return out;
  }

  private original(id: string, withFiles = false) {
    return this.read("messages/" + encodeURIComponent(id) + "?format=full").then(async (m) => ({ m, p: await this.parse(m, withFiles) }));
  }

  private async deliver(m: Message, draft: boolean | undefined, thread?: string): Promise<GmailResult> {
    const raw = mime(m);
    const r = await this.write({ action: draft ? "draft" : "send", raw, thread_id: thread });
    return {
      status: draft ? "draft" : "sent",
      message_id: draft ? r.message?.id : r.id,
      thread_id: draft ? r.message?.threadId : r.threadId,
      draft_id: draft ? r.id : null,
      subject: m.subject,
      to: bare(m.to),
      cc: bare(m.cc),
      bcc: bare(m.bcc),
    };
  }

  private threading(p: Parsed) {
    const id = p.headers["message-id"];
    if (!id) invalid("Source message has no Message-ID");
    return { in_reply_to: safe(id!), references: safe(((p.headers.references || "") + " " + id).trim()) };
  }

  /** New mail. message_id is the old reply path of email_send: recipients and subject exactly as given, no quote. */
  async send(x: Compose & { message_id?: string }): Promise<GmailResult> {
    const text = x.text || "";
    const m: Message = {
      to: addresses(x.to),
      cc: addresses(x.cc),
      bcc: addresses(x.bcc),
      reply_to: addresses(x.reply_to),
      subject: x.subject || "",
      body: text,
      html: looksLikeHtml(text),
      files: checkFiles(x.files),
    };
    if (!x.message_id) return this.deliver(m, x.draft);
    const { m: o, p } = await this.original(x.message_id);
    return this.deliver({ ...m, ...this.threading(p) }, x.draft, o.threadId);
  }

  async reply(id: string, x: Compose, all: boolean): Promise<GmailResult> {
    const files = checkFiles(x.files);
    const { m: o, p } = await this.original(id);
    const h = p.headers;
    const self = new Set([this.account]);
    const from = fromHeader(h["reply-to"] || h.from);
    // Reply-To wins, as in Outlook; without it a reply to own sent mail goes to its recipients, as in Gmail.
    const own = !h["reply-to"] && fromHeader(h.from).some((v) => lower(v) === this.account);
    let to = own ? fromHeader(h.to) : from;
    let cc: string[] = [];
    if (all) {
      to = merge(own ? to : merge(to, fromHeader(h.to)), [], self);
      if (!to.length) to = from;
      cc = merge(fromHeader(h.cc), [], new Set([...self, ...to.map(lower)]));
    }
    to = merge(to, addresses(x.to));
    cc = merge(cc, addresses(x.cc), new Set(to.map(lower)));
    const subject = x.subject || (/^re:/i.test(h.subject || "") ? h.subject! : "Re: " + (h.subject || ""));
    const html = looksLikeHtml(x.text || "");
    const intro = "On " + (h.date || "") + " " + (h.from || "") + " wrote:";
    const quote = html
      ? '<br><div class="gmail_quote"><div>' + htmlEscape(intro) + '</div><blockquote style="margin:0 0 0 .8ex;border-left:1px #ccc solid;padding-left:1ex">' + inner(p) + "</blockquote></div>"
      : NL + NL + intro + NL + plain(p).split(/\r?\n/).map((l) => "> " + l).join(NL);
    return this.deliver(
      { to, cc, bcc: addresses(x.bcc), reply_to: addresses(x.reply_to), subject, ...this.threading(p), body: (x.text || "") + quote, html, files },
      x.draft,
      o.threadId,
    );
  }

  async forward(id: string, x: Compose): Promise<GmailResult> {
    const files = checkFiles(x.files);
    const { m: o, p } = await this.original(id, true);
    const h = p.headers;
    const subject = x.subject || (/^fwd?:/i.test(h.subject || "") ? h.subject! : "Fwd: " + (h.subject || ""));
    const html = looksLikeHtml(x.text || "");
    const lines = ["---------- Forwarded message ---------", "From: " + (h.from || ""), "Date: " + (h.date || ""), "Subject: " + (h.subject || ""), "To: " + (h.to || "")];
    if (h.cc) lines.push("Cc: " + h.cc);
    const quote = html
      ? '<br><br><div class="gmail_quote"><div>' + lines.map(htmlEscape).join("<br>") + "</div><br>" + inner(p) + "</div>"
      : NL + NL + lines.join(NL) + NL + NL + plain(p);
    return this.deliver(
      {
        to: addresses(x.to),
        cc: addresses(x.cc),
        bcc: addresses(x.bcc),
        reply_to: addresses(x.reply_to),
        subject,
        ...this.threading(p),
        body: (x.text || "") + quote,
        html,
        files: [...p.files, ...files],
      },
      x.draft,
      o.threadId,
    );
  }

  private async draft(id: string, withFiles: boolean) {
    const d = await this.read("drafts/" + encodeURIComponent(id) + "?format=full");
    if (!d.message?.id) throw new CommunicationError("NOT_FOUND", "Draft not found");
    return { d, p: await this.parse(d.message, withFiles) };
  }

  /** Recipient lists given replace the draft's; text replaces the author part and keeps the quoted original; files are added. */
  async updateDraft(id: string, x: Compose): Promise<GmailResult> {
    const files = checkFiles(x.files);
    const { d, p } = await this.draft(id, true);
    const h = p.headers;
    let body: string;
    let html: boolean;
    if (x.text === undefined) {
      html = p.html !== undefined;
      body = html ? p.html! : (p.text ?? "");
    } else {
      html = looksLikeHtml(x.text);
      const current = html ? (p.html ?? htmlEscape(p.text ?? "").replace(/\r?\n/g, "<br>")) : (p.text ?? htmlToMarkdown(p.html ?? ""));
      const at = current.search(html ? QUOTE_HTML : QUOTE_TEXT);
      body = x.text + (at < 0 ? "" : (html ? "<br>" : "") + current.slice(at));
    }
    const m: Message = {
      to: x.to ? addresses(x.to) : fromHeader(h.to),
      cc: x.cc ? addresses(x.cc) : fromHeader(h.cc),
      bcc: x.bcc ? addresses(x.bcc) : fromHeader(h.bcc),
      reply_to: x.reply_to ? addresses(x.reply_to) : fromHeader(h["reply-to"]),
      subject: x.subject ?? h.subject ?? "",
      in_reply_to: h["in-reply-to"] ? safe(h["in-reply-to"]) : undefined,
      references: h.references ? safe(h.references) : undefined,
      body,
      html,
      files: [...p.files, ...files],
    };
    const r = await this.write({ action: "update_draft", draft_id: id, raw: mime(m), thread_id: d.message.threadId });
    return {
      status: "draft",
      message_id: r.message?.id,
      thread_id: r.message?.threadId,
      draft_id: r.id || id,
      subject: m.subject,
      to: bare(m.to),
      cc: bare(m.cc),
      bcc: bare(m.bcc),
    };
  }

  async sendDraft(id: string): Promise<GmailResult> {
    const d = await this.read("drafts/" + encodeURIComponent(id) + "?format=metadata");
    const h: Record<string, string> = {};
    for (const v of d.message?.payload?.headers || []) h[String(v.name).toLowerCase()] ??= String(v.value);
    if (!h.to && !h.cc && !h.bcc) invalid("Draft has no recipients");
    const r = await this.write({ action: "send_draft", draft_id: id });
    return {
      status: "sent",
      message_id: r.id,
      thread_id: r.threadId,
      draft_id: null,
      subject: h.subject || "",
      to: bare(fromHeader(h.to)),
      cc: bare(fromHeader(h.cc)),
      bcc: bare(fromHeader(h.bcc)),
    };
  }

  async deleteDraft(id: string): Promise<GmailResult> {
    await this.write({ action: "delete_draft", draft_id: id });
    return { status: "deleted" };
  }
}

/** Original body for an HTML quote: its HTML without document wrappers, or its text escaped. */
function inner(p: Parsed): string {
  if (p.html !== undefined)
    return p.html.replace(/<head[\s\S]*?<\/head>/i, "").replace(/<\/?(html|body)[^>]*>/gi, "");
  return htmlEscape(p.text ?? "").replace(/\r?\n/g, "<br>");
}
const plain = (p: Parsed) => p.text ?? htmlToMarkdown(p.html ?? "");
