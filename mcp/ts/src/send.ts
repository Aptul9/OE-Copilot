// Mail writes: send, reply, forward, drafts, manage (port of outlook_api.py).

import fs from "node:fs";
import path from "node:path";
import { q, resolveFolderId, type Obj, type Outlook } from "./api.ts";
import { formatBodyForHtml, looksLikeHtml } from "./html.ts";

export type Recipient = { EmailAddress: { Address: string; Name?: string } };
export type Recipients = string | (string | Obj)[] | undefined | null;

export function parseRecipients(val: Recipients): Recipient[] {
  if (!val) return [];
  const parts: (string | Obj)[] = [];
  for (const item of Array.isArray(val) ? val : [val]) {
    if (typeof item === "string" && !item.includes("<") && /[,;]/.test(item)) parts.push(...item.split(/[,;]/).map((s) => s.trim()).filter(Boolean));
    else parts.push(item);
  }
  const out: Recipient[] = [];
  const seen = new Set<string>();
  for (const p of parts) {
    let r: Recipient | null = null;
    if (p && typeof p === "object") {
      if (p.EmailAddress?.Address) r = p as Recipient;
      else if (p.Address) r = { EmailAddress: { Address: String(p.Address), Name: String(p.Name ?? "") } };
      else if (p.address) r = { EmailAddress: { Address: String(p.address), Name: String(p.name ?? "") } };
    } else if (typeof p === "string") {
      const m = /<([^>]+)>/.exec(p);
      if (m) {
        const name = p.slice(0, m.index).trim().replace(/^["']+|["']+$/g, "");
        r = { EmailAddress: name ? { Address: m[1]!.trim(), Name: name } : { Address: m[1]!.trim() } };
      } else if (p.trim()) r = { EmailAddress: { Address: p.trim() } };
    }
    const key = r?.EmailAddress.Address.trim().toLowerCase();
    if (r && key && !seen.has(key)) {
      seen.add(key);
      out.push(r);
    }
  }
  return out;
}

export const addressSet = (val: Recipients) => new Set(parseRecipients(val).map((r) => r.EmailAddress.Address.trim().toLowerCase()));
const addr = (r: Obj) => String(r?.EmailAddress?.Address ?? "").trim().toLowerCase();
const addrList = (recs: Obj[]) => recs.map((r) => r?.EmailAddress?.Address).filter(Boolean) as string[];

function mergeRecipients(existing: Obj[] = [], extra: Obj[] = []): Obj[] {
  const out = [...existing];
  const seen = new Set(out.map(addr).filter(Boolean));
  for (const r of extra) {
    const k = addr(r);
    if (k && !seen.has(k)) {
      seen.add(k);
      out.push(r);
    }
  }
  return out;
}

const withoutExcluded = (recs: Obj[], excluded: Set<string>) => (excluded.size ? recs.filter((r) => !excluded.has(addr(r))) : recs);

/** A file sent with the request (remote server: the caller's disk is not reachable). */
export interface InlineFile {
  name: string;
  content_base64: string;
  content_type?: string;
}

const BASE64 = /^[A-Za-z0-9+/]*={0,2}$/;

/** Local files and inline files as FileAttachment objects; a missing or malformed file stops the send before anything is created. */
export function attachmentObjects(files: string[] = [], inline: InlineFile[] = []): Obj[] {
  const local = files.map((f) => {
    const p = f.trim().replace(/^~(?=$|[\\/])/, process.env.USERPROFILE || process.env.HOME || "~");
    if (!fs.existsSync(p) || !fs.statSync(p).isFile()) throw new Error(`Attachment file not found: ${p}`);
    return { "@odata.type": "#Microsoft.OutlookServices.FileAttachment", Name: path.basename(p), ContentBytes: fs.readFileSync(p).toString("base64") };
  });
  const sent = inline.map((f) => {
    const name = path.basename(f.name.replace(/\\/g, "/")).trim();
    const bytes = f.content_base64.replace(/\s+/g, "");
    if (!name) throw new Error("Attachment name is empty");
    if (!bytes || bytes.length % 4 !== 0 || !BASE64.test(bytes)) throw new Error(`Attachment ${name}: content_base64 is not base64`);
    const a: Obj = { "@odata.type": "#Microsoft.OutlookServices.FileAttachment", Name: name, ContentBytes: bytes };
    if (f.content_type) a.ContentType = f.content_type;
    return a;
  });
  return [...local, ...sent];
}

const importanceOf = (v?: string) => {
  const c = v ? v[0]!.toUpperCase() + v.slice(1).toLowerCase() : "";
  return ["Low", "Normal", "High"].includes(c) ? c : undefined;
};

export interface SendOptions {
  to?: Recipients;
  cc?: Recipients;
  bcc?: Recipients;
  reply_to?: Recipients;
  /** Reply and reply-all: to and cc are added to the recipients Outlook proposes instead of replacing them. */
  add_recipients?: boolean;
  subject?: string;
  body?: string;
  is_html?: boolean;
  reply_to_id?: string;
  reply_all?: boolean;
  forward_to_id?: string;
  replace_cc?: boolean;
  exclude_recipients?: Recipients;
  importance?: string;
  attachments?: string[];
  files?: InlineFile[];
  save_as_draft?: boolean;
}

export interface SendResult {
  status: "sent" | "draft";
  type?: string;
  target_message_id?: string;
  draft_id?: string | null;
  subject: string;
  to: string[];
  cc: string[];
  bcc: string[];
}

async function viaDraft(o: Outlook, account: string | undefined, targetId: string, action: string, opt: SendOptions): Promise<SendResult> {
  const atts = attachmentObjects(opt.attachments, opt.files);
  const draft = await o.request(account, `me/messages/${q(targetId)}/${action}`, "POST", { Comment: formatBodyForHtml(opt.body ?? "", opt.is_html) });
  const draftId = draft.Id as string | undefined;
  if (!draftId) throw new Error(`Failed to create draft via ${action} on message ${targetId}`);
  const draftPath = `me/messages/${q(draftId)}`;
  const excluded = addressSet(opt.exclude_recipients);
  const to = parseRecipients(opt.to);
  const cc = parseRecipients(opt.cc);
  let toR: Obj[] = opt.add_recipients ? mergeRecipients(draft.ToRecipients, to) : to.length ? to : (draft.ToRecipients ?? []);
  let ccR: Obj[] = opt.add_recipients
    ? mergeRecipients(draft.CcRecipients, cc)
    : opt.replace_cc || action !== "createreplyall"
      ? cc.length
        ? cc
        : opt.replace_cc
          ? []
          : (draft.CcRecipients ?? [])
      : mergeRecipients(draft.CcRecipients, cc);
  let bccR = mergeRecipients(draft.BccRecipients, parseRecipients(opt.bcc));
  toR = withoutExcluded(toR, excluded);
  ccR = withoutExcluded(ccR, opt.add_recipients ? new Set([...excluded, ...toR.map(addr)]) : excluded);
  bccR = withoutExcluded(bccR, excluded);
  if (!toR.length && !opt.save_as_draft) {
    await o.request(account, draftPath, "DELETE");
    throw new Error("No 'To' recipients left after applying recipient filters. Specify 'to' explicitly.");
  }
  const patch: Obj = { ToRecipients: toR, CcRecipients: ccR, BccRecipients: bccR };
  const replyTo = parseRecipients(opt.reply_to);
  if (replyTo.length) patch.ReplyTo = replyTo;
  if (opt.subject?.trim()) patch.Subject = opt.subject.trim();
  const imp = importanceOf(opt.importance);
  if (imp) patch.Importance = imp;
  try {
    await o.request(account, draftPath, "PATCH", patch);
    for (const a of atts) await o.request(account, `${draftPath}/attachments`, "POST", a);
    if (!opt.save_as_draft) await o.request(account, `${draftPath}/send`, "POST");
  } catch (e) {
    if (!opt.save_as_draft) await o.request(account, draftPath, "DELETE").catch(() => {});
    throw e;
  }
  return {
    status: opt.save_as_draft ? "draft" : "sent",
    type: action,
    target_message_id: targetId,
    draft_id: opt.save_as_draft ? draftId : null,
    subject: String(opt.subject || draft.Subject || "").trim(),
    to: addrList(toR),
    cc: addrList(ccR),
    bcc: addrList(bccR),
  };
}

export async function sendEmail(o: Outlook, account: string | undefined, opt: SendOptions): Promise<SendResult> {
  if (opt.reply_to_id) return viaDraft(o, account, opt.reply_to_id, opt.reply_all ? "createreplyall" : "createreply", opt);
  if (opt.forward_to_id) {
    if (!opt.to && !opt.save_as_draft) throw new Error("Recipient ('to') is required when forwarding an email");
    return viaDraft(o, account, opt.forward_to_id, "createforward", { ...opt, replace_cc: true });
  }
  if (!opt.to && !opt.save_as_draft) throw new Error("Recipient (to) is required for new email");
  const toR = parseRecipients(opt.to);
  const ccR = parseRecipients(opt.cc);
  const bccR = parseRecipients(opt.bcc);
  const body = opt.body ?? "";
  const message: Obj = {
    Subject: opt.subject ?? "",
    Body: { ContentType: opt.is_html || looksLikeHtml(body) ? "HTML" : "Text", Content: body },
    ToRecipients: toR,
  };
  if (ccR.length) message.CcRecipients = ccR;
  if (bccR.length) message.BccRecipients = bccR;
  const replyTo = parseRecipients(opt.reply_to);
  if (replyTo.length) message.ReplyTo = replyTo;
  const imp = importanceOf(opt.importance);
  if (imp) message.Importance = imp;
  const atts = attachmentObjects(opt.attachments, opt.files);
  const summary = { subject: opt.subject ?? "", to: addrList(toR), cc: addrList(ccR), bcc: addrList(bccR) };
  if (opt.save_as_draft) {
    const draft = await o.request(account, "me/messages", "POST", message);
    for (const a of atts) await o.request(account, `me/messages/${q(draft.Id)}/attachments`, "POST", a);
    return { status: "draft", draft_id: draft.Id, ...summary };
  }
  if (atts.length) message.Attachments = atts;
  await o.request(account, "me/sendmail", "POST", { Message: message, SaveToSentItems: "true" });
  return { status: "sent", ...summary };
}

// Outlook puts the quoted original of a reply or forward after this marker; edits keep everything from it on.
// Stored drafts carry the id with an x_ prefix.
const QUOTE = /<div id="appendonsend"><\/div>|<hr[^>]*>\s*<div[^>]*id="(?:x_)?divRplyFwdMsg"|<div[^>]*id="(?:x_)?divRplyFwdMsg"/i;

/** New author text over an existing draft body; the quoted original and the HTML head survive. */
export function replaceDraftBody(body: Obj | undefined, text: string): Obj {
  const html = String(body?.Content ?? "");
  const at = String(body?.ContentType).toLowerCase() === "html" ? html.search(QUOTE) : -1;
  if (at < 0) return { ContentType: looksLikeHtml(text) ? "HTML" : "Text", Content: text };
  const open = /<body[^>]*>/i.exec(html.slice(0, at));
  const head = open ? html.slice(0, open.index + open[0].length) : "";
  return { ContentType: "HTML", Content: head + formatBodyForHtml(text) + html.slice(at) };
}

export interface DraftUpdate {
  to?: Recipients;
  cc?: Recipients;
  bcc?: Recipients;
  reply_to?: Recipients;
  subject?: string;
  body?: string;
  files?: InlineFile[];
}

async function loadDraft(o: Outlook, account: string | undefined, id: string): Promise<Obj> {
  const d = await o.request(account, `me/messages/${q(id)}?$select=IsDraft,Subject,Body,ToRecipients,CcRecipients,BccRecipients`);
  if (!d.IsDraft) throw new Error("Message is not a draft");
  return d;
}

const draftResult = (status: SendResult["status"], id: string | null, d: Obj): SendResult => ({
  status,
  draft_id: id,
  subject: String(d.Subject ?? ""),
  to: addrList(d.ToRecipients ?? []),
  cc: addrList(d.CcRecipients ?? []),
  bcc: addrList(d.BccRecipients ?? []),
});

/** Recipient lists given replace the draft's; text replaces the author part and keeps the quoted original; files are added. */
export async function updateDraft(o: Outlook, account: string | undefined, id: string, opt: DraftUpdate): Promise<SendResult> {
  const atts = attachmentObjects([], opt.files);
  const d = await loadDraft(o, account, id);
  const patch: Obj = {};
  if (opt.to) patch.ToRecipients = parseRecipients(opt.to);
  if (opt.cc) patch.CcRecipients = parseRecipients(opt.cc);
  if (opt.bcc) patch.BccRecipients = parseRecipients(opt.bcc);
  if (opt.reply_to) patch.ReplyTo = parseRecipients(opt.reply_to);
  if (opt.subject !== undefined) patch.Subject = opt.subject;
  if (opt.body !== undefined) patch.Body = replaceDraftBody(d.Body, opt.body);
  const after = Object.keys(patch).length ? await o.request(account, `me/messages/${q(id)}`, "PATCH", patch) : d;
  for (const a of atts) await o.request(account, `me/messages/${q(id)}/attachments`, "POST", a);
  return draftResult("draft", id, { ...d, ...patch, ...after });
}

export async function sendDraft(o: Outlook, account: string | undefined, id: string): Promise<SendResult> {
  const d = await loadDraft(o, account, id);
  if (!d.ToRecipients?.length && !d.CcRecipients?.length && !d.BccRecipients?.length) throw new Error("Draft has no recipients");
  await o.request(account, `me/messages/${q(id)}/send`, "POST");
  return draftResult("sent", null, d);
}

export async function deleteDraft(o: Outlook, account: string | undefined, id: string): Promise<{ status: "deleted" }> {
  await loadDraft(o, account, id);
  await o.request(account, `me/messages/${q(id)}`, "DELETE");
  return { status: "deleted" };
}

export const MANAGE_ACTIONS = ["mark_read", "mark_unread", "flag", "unflag", "archive", "delete", "move"] as const;
export type ManageAction = (typeof MANAGE_ACTIONS)[number];

export async function manageEmail(o: Outlook, account: string | undefined, ids: string | string[], action: string, destination?: string): Promise<Obj[]> {
  const list = (Array.isArray(ids) ? ids : [ids]).filter(Boolean);
  if (!list.length) throw new Error("message_id is required");
  const act = action.trim().toLowerCase() as ManageAction;
  if (!MANAGE_ACTIONS.includes(act)) throw new Error(`Unsupported action '${action}'. Use: ${MANAGE_ACTIONS.join(", ")}.`);
  let dest: string | null = null;
  if (act === "archive") dest = "Archive";
  else if (act === "delete") dest = "DeletedItems";
  else if (act === "move") {
    if (!destination) throw new Error("destination_folder is required when action is move");
    dest = await resolveFolderId(o, account, destination);
    if (!dest) throw new Error(`Destination folder '${destination}' not found`);
  }
  const patches: Partial<Record<ManageAction, [Obj, string]>> = {
    mark_read: [{ IsRead: true }, "marked_read"],
    mark_unread: [{ IsRead: false }, "marked_unread"],
    flag: [{ Flag: { FlagStatus: "Flagged" } }, "flagged"],
    unflag: [{ Flag: { FlagStatus: "NotFlagged" } }, "unflagged"],
  };
  const out: Obj[] = [];
  for (const id of list) {
    const p = `me/messages/${q(id)}`;
    const patch = patches[act];
    if (patch) {
      await o.request(account, p, "PATCH", patch[0]);
      out.push({ id, status: patch[1] });
    } else {
      const moved = await o.request(account, `${p}/move`, "POST", { DestinationId: dest });
      out.push({ id, new_id: moved.Id ?? id, status: `moved_to_${dest}` });
    }
  }
  return out;
}
