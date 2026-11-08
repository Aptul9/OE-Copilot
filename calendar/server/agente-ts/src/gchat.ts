// Binary Google Chat RPCs used by gweb. No browser, stream listener, implicit read receipts or send retries.
import crypto from "node:crypto";
import fs from "node:fs";
import protobuf from "protobufjs";
import { assorbiCookie, intestazioneCookie, leggiSessioneImportata, ruotaCookie, type SessioneGoogleWeb } from "./google-web.ts";
import type { FetchFn } from "./outlook-rest.ts";

type ObjectData = Record<string, unknown>;
const obj = (v: unknown): ObjectData => v !== null && typeof v === "object" && !Array.isArray(v) ? v as ObjectData : {};
const array = (v: unknown): ObjectData[] => Array.isArray(v) ? v.map(obj) : [];
const text = (v: unknown): string => typeof v === "string" || typeof v === "number" ? String(v) : "";
const root = protobuf.parse(fs.readFileSync(new URL("../proto/googlechat.proto", import.meta.url), "utf8"), { keepCase: true }).root;
const BASE = "https://chat.google.com/u/0";
const API_KEY = "AIzaSyD7InnYR3VKdb4j2rMUEbTCIr2VyEazl6k";
export const CHAT_MAX_BYTES = 8 * 1024 * 1024;
const ATTACHMENT_MAX_BYTES = 5 * 1024 * 1024;
const HEADER = { client_type: "WEB", client_version: "2440378181258", client_feature_capabilities: { spam_room_invites_level: "FULLY_SUPPORTED" } };

export function encodeChat(type: string, value: ObjectData): Uint8Array {
  const schema = root.lookupType(type);
  for (const key of Object.keys(value)) if (!schema.fields[key]) throw new Error(`${type}: unknown field ${key}`);
  const message = schema.fromObject(value);
  const error = schema.verify(message);
  if (error) throw new Error(`${type}: ${error}`);
  return schema.encode(message).finish();
}

export function decodeChat(type: string, bytes: Uint8Array): ObjectData {
  const schema = root.lookupType(type);
  return schema.toObject(schema.decode(bytes), { longs: String, enums: String, bytes: String }) as ObjectData;
}

export class GoogleChatError extends Error {
  readonly status: number;
  constructor(message: string, status = 502) { super(message); this.status = status; }
}

export interface Conversation {
  id: string;
  kind: "space" | "dm";
  name: string;
  unread: number;
  last: string;
}

export interface ChatMessage {
  id: string;
  thread: string;
  time: string;
  from: string;
  from_id: string;
  text: string;
  reply: boolean;
  edited: boolean;
  deleted: boolean;
  attachments: { token: string; name: string; mime_type: string }[];
  items: ObjectData[];
  reactions: ObjectData[];
}

function id(value: unknown, name: string): string {
  if (typeof value !== "string" || !/^[A-Za-z0-9_%=-]{1,256}$/.test(value)) throw new GoogleChatError(`invalid ${name}`, 400);
  return value;
}

function group(conversation: string, kind: "space" | "dm"): ObjectData {
  id(conversation, "conversation");
  return kind === "dm" ? { dm_id: { dm_id: conversation } } : { space_id: { space_id: conversation } };
}

function iso(micros: unknown): string {
  const ms = Number(text(micros)) / 1000;
  return Number.isFinite(ms) && ms >= 0 && ms < 8.64e15 ? new Date(ms).toISOString() : "";
}

function trustedUrl(raw: string): URL {
  let u: URL;
  try { u = new URL(raw); } catch { throw new GoogleChatError("invalid Google URL", 400); }
  if (u.protocol !== "https:" || u.username || u.password || u.hash || (u.port && u.port !== "443") ||
      !["chat.google.com", "accounts.google.com", "chat-wsx.usercontent.google.com"].includes(u.hostname) && !u.hostname.endsWith(".googleusercontent.com")) {
    throw new GoogleChatError("Google URL host not allowed", 400);
  }
  return u;
}

async function bytes(response: Response, max = CHAT_MAX_BYTES): Promise<Buffer> {
  const size = Number(response.headers.get("content-length"));
  if (size > max) { await response.body?.cancel(); throw new GoogleChatError("Google response exceeds size limit"); }
  const chunks: Uint8Array[] = [];
  let length = 0;
  if (response.body) for await (const chunk of response.body) {
    length += chunk.byteLength;
    if (length > max) throw new GoogleChatError("Google response exceeds size limit");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

export class GoogleChat {
  readonly session: SessioneGoogleWeb;
  private readonly fetchFn: FetchFn;
  private readonly now: () => number;
  private readonly changed: () => void;
  private xsrf = "";
  private xsrfAt = 0;
  private refreshing: Promise<void> | undefined;
  private renewing: Promise<void> | undefined;
  private self = "";
  private readonly names = new Map<string, { name: string; email: string }>();

  constructor(session: SessioneGoogleWeb, options: { fetchFn?: FetchFn; now?: () => number; changed?: () => void } = {}) {
    this.session = session;
    this.fetchFn = options.fetchFn ?? fetch;
    this.now = options.now ?? Date.now;
    this.changed = options.changed ?? (() => {});
    // Saved files pass the same boundary as imported files before any cookie leaves.
    const validated = leggiSessioneImportata(session, Math.floor(this.now() / 1000));
    session.cookies = validated.cookies;
    session.user_agent = validated.user_agent;
  }

  private async fetch(url: string, init: RequestInit = {}): Promise<Response> {
    const u = trustedUrl(url);
    const headers = new Headers(init.headers);
    headers.set("User-Agent", this.session.user_agent);
    const cookies = intestazioneCookie(this.session.cookies, u.hostname, u.pathname, Math.floor(this.now() / 1000));
    if (cookies) headers.set("Cookie", cookies);
    else headers.delete("Cookie");
    let response: Response;
    try { response = await this.fetchFn(u.href, { ...init, headers, redirect: "manual", signal: AbortSignal.timeout(30_000) }); }
    catch (error) {
      if (error instanceof GoogleChatError) throw error;
      throw new GoogleChatError("Google Chat transport failed; write outcome may be unknown");
    }
    if (assorbiCookie(this.session.cookies, u.hostname, response.headers.getSetCookie(), Math.floor(this.now() / 1000))) this.changed();
    return response;
  }

  private async xsrfOnce(): Promise<void> {
    const hs = '["h_hs",null,null,[1,0],null,null,"gmail.pinto-server_20230730.06_p0",1,null,[15,38,36,35,26,30,41,18,24,11,21,14,6],null,null,"3Mu86PSulM4.en..es5",0,null,null,[0]]';
    const q = new URLSearchParams({ origin: "https://mail.google.com", shell: "9", hl: "en", wfi: "gtn-roster-iframe-id", hs });
    const response = await this.fetch(`${BASE}/mole/world?${q}`, { headers: { refer: "https://mail.google.com/" } });
    if (response.status === 401 || response.status === 302 || response.status === 303) { await response.body?.cancel(); throw new GoogleChatError("Google session expired: import session again", 409); }
    if (!response.ok) { await response.body?.cancel(); throw new GoogleChatError(`Google Chat HTTP ${response.status}`); }
    const html = (await bytes(response)).toString("utf8");
    const match = /window\.WIZ_global_data\s*=\s*({[^]*?});<\/script>/.exec(html);
    if (!match) throw new GoogleChatError("Google Chat session page not recognized");
    let data: ObjectData;
    try { data = obj(JSON.parse(match[1]!)); } catch { throw new GoogleChatError("Google Chat session page not recognized"); }
    if (data.qwAQke === "AccountsSignInUi") throw new GoogleChatError("Google session expired: import session again", 409);
    if (typeof data.SMqcke !== "string" || !data.SMqcke) throw new GoogleChatError("Google Chat XSRF token missing");
    this.xsrf = data.SMqcke;
    this.xsrfAt = this.now();
    this.self = "";
  }

  private async ensureXsrf(force = false): Promise<void> {
    if (!force && this.xsrf && this.now() - this.xsrfAt < 86_400_000) return;
    if (this.refreshing) return this.refreshing;
    this.refreshing = (async () => {
      try { await this.xsrfOnce(); }
      catch (e) {
        if (!(e instanceof GoogleChatError) || e.status !== 409) throw e;
        await this.renewSession();
      }
    })().finally(() => { this.refreshing = undefined; });
    return this.refreshing;
  }

  private renewSession(): Promise<void> {
    if (this.renewing) return this.renewing;
    this.renewing = (async () => {
      if (await ruotaCookie(this.session, this.fetchFn, Math.floor(this.now() / 1000))) this.changed();
      this.self = "";
      await this.xsrfOnce();
    })().finally(() => { this.renewing = undefined; });
    return this.renewing;
  }

  private async rpc(endpoint: string, requestType: string, responseType: string, data: ObjectData): Promise<ObjectData> {
    const body = encodeChat(requestType, { ...data, request_header: HEADER });
    await this.ensureXsrf();
    if (endpoint !== "get_self_user_status" && this.session.self_id) await this.selfId();
    for (const retry of [false, true]) {
      const q = new URLSearchParams({ c: "1", rt: "b", alt: "proto", key: API_KEY });
      const response = await this.fetch(`${BASE}/api/${endpoint}?${q}`, {
        method: "POST", headers: { "Content-Type": "application/x-protobuf", "X-Goog-Encode-Response-If-Executable": "base64", "x-framework-xsrf-token": this.xsrf }, body: Buffer.from(body),
      });
      if (response.status === 401) {
        await response.body?.cancel();
        if (retry) throw new GoogleChatError("Google session rejected: import session again", 409);
        await this.renewSession();
        if (endpoint !== "get_self_user_status" && this.session.self_id) await this.selfId();
        continue;
      }
      if (!response.ok) { await response.body?.cancel(); throw new GoogleChatError(`Google Chat HTTP ${response.status}`, [400, 403, 404, 429].includes(response.status) ? response.status : 502); }
      const data = await bytes(response);
      try { return decodeChat(responseType, data); }
      catch {
        try {
          const encoded = data.toString("ascii").trim();
          if (!/^[A-Za-z0-9+/]+={0,2}$/.test(encoded)) throw new Error();
          return decodeChat(responseType, Buffer.from(encoded, "base64"));
        } catch { throw new GoogleChatError("Google Chat returned invalid protobuf; write outcome may be unknown"); }
      }
    }
    throw new GoogleChatError("Google session rejected", 409);
  }

  private async selfId(): Promise<string> {
    if (!this.self) {
      const response = await this.rpc("get_self_user_status", "GetSelfUserStatusRequest", "GetSelfUserStatusResponse", {});
      this.self = text(obj(obj(response.user_status).user_id).id);
      if (!this.self) throw new GoogleChatError("Google Chat identity missing");
    }
    if (this.session.self_id && this.session.self_id !== this.self) throw new GoogleChatError("Google Chat session belongs to another account", 409);
    return this.self;
  }

  private async resolve(ids: string[]): Promise<void> {
    const need = [...new Set(ids)].filter(id => id && !this.names.has(id));
    for (let i = 0; i < need.length; i += 40) {
      const response = await this.rpc("get_members", "GetMembersRequest", "GetMembersResponse", { member_ids: need.slice(i, i + 40).map(id => ({ user_id: { id } })) });
      for (const member of array(response.members)) {
        const user = obj(member.user);
        const userId = text(obj(user.user_id).id);
        if (userId) this.names.set(userId, { name: text(user.name) || text(user.email) || userId, email: text(user.email).toLowerCase() });
      }
    }
  }

  async identity(): Promise<{ account: string; self_id: string }> {
    const self = await this.selfId();
    await this.resolve([self]);
    const account = this.names.get(self)?.email ?? "";
    if (!account || account !== this.session.account.toLowerCase()) throw new GoogleChatError("Google Chat session belongs to another account or has no email", 409);
    return { account, self_id: self };
  }

  async list(): Promise<{ conversations: Conversation[]; self_id: string; coverage: string; pagination: unknown[] }> {
    const world = await this.rpc("paginated_world", "PaginatedWorldRequest", "PaginatedWorldResponse", {
      fetch_from_user_spaces: true, fetch_options: ["EXCLUDE_GROUP_LITE"], world_section_requests: [{ page_size: 200 }],
    });
    const self = await this.selfId();
    const items = array(world.world_items);
    await this.resolve(items.flatMap(item => array(obj(item.dm_members).members).map(u => text(u.id))));
    const conversations: Conversation[] = items.map(item => {
      const g = obj(item.group_id);
      const dm = Boolean(g.dm_id);
      const id = text(dm ? obj(g.dm_id).dm_id : obj(g.space_id).space_id);
      const peers = array(obj(item.dm_members).members).map(u => text(u.id)).filter(id => id !== self);
      return { id, kind: dm ? "dm" : "space", name: dm ? peers.map(id => this.names.get(id)?.name ?? id).join(", ") : text(item.room_name) || id,
        unread: Number(obj(item.read_state).unread_message_count ?? 0), last: iso(item.sort_timestamp) };
    });
    return { conversations: conversations.filter(c => c.id).sort((a, b) => b.last.localeCompare(a.last)), self_id: self, coverage: "recent-conversations", pagination: array(world.world_section_responses) };
  }

  async read(conversation: string, kind: "space" | "dm", topics = 100): Promise<{ messages: ChatMessage[]; self_id: string; coverage: string; limits: ObjectData }> {
    if (!Number.isInteger(topics) || topics < 1 || topics > 1000) throw new GoogleChatError("topics must be 1..1000", 400);
    const response = await this.rpc("list_topics", "ListTopicsRequest", "ListTopicsResponse", { group_id: group(conversation, kind), page_size_for_topics: topics, page_size_for_unread_replies: 100, page_size_for_read_replies: 20 });
    const rows = array(response.topics).flatMap(topic => array(topic.replies).map(m => ({ m, thread: text(obj(topic.id).topic_id) })));
    await this.resolve(rows.map(({ m }) => text(obj(obj(m.creator).user_id).id)));
    const messages = rows.map(({ m, thread }): ChatMessage => {
      const id = text(obj(m.id).message_id);
      const fromId = text(obj(obj(m.creator).user_id).id);
      const annotations = array(m.annotations);
      return { id, thread, from_id: fromId, from: this.names.get(fromId)?.name ?? fromId, time: iso(m.create_time), text: Number(m.delete_time) > 0 ? "[deleted]" : text(m.text_body),
        reply: id !== thread, edited: Number(m.last_edit_time) > 0, deleted: Number(m.delete_time) > 0,
        attachments: annotations.filter(a => a.upload_metadata).map(a => { const u = obj(a.upload_metadata); return { token: text(u.attachment_token), name: text(u.content_name), mime_type: text(u.content_type) }; }),
        items: annotations.filter(a => a.drive_metadata || a.url_metadata).map(a => obj(a.drive_metadata ?? a.url_metadata)), reactions: array(m.reactions) };
    }).sort((a, b) => a.time.localeCompare(b.time));
    return { messages, self_id: await this.selfId(), coverage: "recent-topics", limits: { topics, unread_replies: 100, read_replies: 20 } };
  }

  async search(query: string, topics = 100): Promise<ObjectData> {
    if (!query || query.length > 1024) throw new GoogleChatError("query must be 1..1024 characters", 400);
    const list = await this.list();
    const hits = [];
    for (const conversation of list.conversations) {
      const read = await this.read(conversation.id, conversation.kind, topics);
      for (const message of read.messages) if (message.text.toLowerCase().includes(query.toLowerCase())) hits.push({ conversation, message });
    }
    return { hits, coverage: "local-search-recent-topics", topics_per_conversation: topics };
  }

  async details(conversation: string, kind: "space" | "dm"): Promise<ObjectData> {
    await this.selfId();
    return this.rpc("get_group", "GetGroupRequest", "GetGroupResponse", { group_id: group(conversation, kind), fetch_options: ["MEMBERS", "MEMBER_IDS_ONLY"] });
  }

  async download(url: string): Promise<{ data: string; mime_type: string }> {
    for (let hop = 0; hop < 8; hop++) {
      if (trustedUrl(url).hostname === "accounts.google.com") throw new GoogleChatError("Google attachment sign-in required", 409);
      const response = await this.fetch(url);
      if ([301, 302, 303, 307, 308].includes(response.status)) {
        await response.body?.cancel();
        const location = response.headers.get("location");
        if (!location) throw new GoogleChatError("Google attachment redirect missing URL");
        url = trustedUrl(new URL(location, url).href).href;
        continue;
      }
      if (!response.ok) { await response.body?.cancel(); throw new GoogleChatError(`Google attachment HTTP ${response.status}`); }
      return { data: (await bytes(response, ATTACHMENT_MAX_BYTES)).toString("base64"), mime_type: response.headers.get("content-type") ?? "application/octet-stream" };
    }
    throw new GoogleChatError("too many Google attachment redirects");
  }

  async attachment(token: string, mimeType: string): Promise<{ data: string; mime_type: string }> {
    if (!token || token.length > 8192 || mimeType.length > 256) throw new GoogleChatError("invalid attachment", 400);
    const query = new URLSearchParams({ attachment_token: token, url_type: mimeType.startsWith("image/") ? "FIFE_URL" : "DOWNLOAD_URL" });
    if (mimeType.startsWith("image/")) { query.set("sz", "w10000-h10000"); query.set("content_type", mimeType); }
    return this.download(`https://chat.google.com/api/get_attachment_url?${query}`);
  }

  private async upload(conversation: string, file: ObjectData): Promise<ObjectData> {
    if (typeof file.name !== "string" || !/^[^\x00-\x1f\x7f/\\]{1,180}$/.test(file.name) || typeof file.data !== "string" || !/^[A-Za-z0-9+/]*={0,2}$/.test(file.data)) throw new GoogleChatError("invalid file", 400);
    const data = Buffer.from(file.data, "base64");
    if (data.length > ATTACHMENT_MAX_BYTES) throw new GoogleChatError("file exceeds size limit", 413);
    const mime = typeof file.mime_type === "string" && /^[\w.+-]+\/[\w.+-]+$/.test(file.mime_type) ? file.mime_type : "application/octet-stream";
    await this.ensureXsrf();
    await this.selfId();
    const response = await this.fetch(`https://chat.google.com/uploads?${new URLSearchParams({ group_id: conversation })}`, { method: "POST", headers: {
      "x-goog-upload-protocol": "resumable", "x-goog-upload-command": "start", "x-goog-upload-content-length": String(data.length), "x-goog-upload-content-type": mime,
      "x-goog-upload-file-name": encodeURIComponent(file.name), "x-framework-xsrf-token": this.xsrf,
    } });
    await response.body?.cancel();
    const next = response.headers.get("x-goog-upload-url");
    if (!response.ok || !next || trustedUrl(next).hostname !== "chat.google.com") throw new GoogleChatError("Google upload start failed");
    await this.ensureXsrf();
    await this.selfId();
    const done = await this.fetch(next, { method: "PUT", headers: { "x-goog-upload-command": "upload, finalize", "x-goog-upload-protocol": "resumable", "x-goog-upload-offset": "0", "x-framework-xsrf-token": this.xsrf }, body: data });
    if (!done.ok) { await done.body?.cancel(); throw new GoogleChatError(`Google upload HTTP ${done.status}`); }
    let metadata: ObjectData;
    try { metadata = decodeChat("UploadMetadata", Buffer.from((await bytes(done)).toString("ascii").replace(/\s/g, ""), "base64")); }
    catch { throw new GoogleChatError("Google upload returned invalid metadata"); }
    if (!metadata.attachment_token) throw new GoogleChatError("Google upload returned no attachment token");
    return { type: "UPLOAD_METADATA", upload_metadata: metadata };
  }

  async write(data: ObjectData): Promise<ObjectData> {
    if (data.confirm !== true) throw new GoogleChatError("confirm: true required", 400);
    const fields: Record<string, string[]> = {
      send: ["text", "thread", "files"], mark_read: [], edit: ["message_id", "thread", "text", "topics"],
      delete: ["message_id", "thread", "topics"], react: ["message_id", "thread", "emoji", "remove", "topics"],
    };
    if (typeof data.action !== "string" || !Object.hasOwn(fields, data.action)) throw new GoogleChatError("unknown Chat action", 400);
    const allowed = ["account", "action", "confirm", "conversation", "kind", ...fields[data.action]!];
    if (Object.keys(data).some(key => !allowed.includes(key))) throw new GoogleChatError("unexpected Chat write field", 400);
    const conversation = id(data.conversation, "conversation");
    if (data.kind !== "space" && data.kind !== "dm") throw new GoogleChatError("kind must be space or dm", 400);
    const g = group(conversation, data.kind);
    const action = data.action;
    const messageInfo = { accept_format_annotations: true };
    if (data.text !== undefined && typeof data.text !== "string" || data.files !== undefined && !Array.isArray(data.files) || data.thread !== undefined && typeof data.thread !== "string" || data.remove !== undefined && typeof data.remove !== "boolean") throw new GoogleChatError("invalid Chat write field type", 400);
    await this.selfId();
    if (action === "send") {
      const body = typeof data.text === "string" ? data.text : "";
      const files = array(data.files);
      if ((!body && !files.length) || body.length > 32000 || files.length > 10) throw new GoogleChatError("empty or oversized message", 400);
      const annotations = [];
      for (const file of files) annotations.push(await this.upload(conversation, file));
      const base = { local_id: `oe%${crypto.randomBytes(8).toString("hex")}`, text_body: body, annotations, message_info: messageInfo };
      if (data.thread) return this.rpc("create_message", "CreateMessageRequest", "CreateMessageResponse", { ...base, parent_id: { topic_id: { group_id: g, topic_id: id(data.thread, "thread") } } });
      return this.rpc("create_topic", "CreateTopicRequest", "CreateTopicResponse", { ...base, group_id: g, history_v2: true });
    }
    if (action === "mark_read") return this.rpc("mark_group_readstate", "MarkGroupReadstateRequest", "MarkGroupReadstateResponse", { id: g, last_read_time: String(this.now() * 1000) });
    const messageId = id(data.message_id, "message_id");
    let thread = typeof data.thread === "string" ? id(data.thread, "thread") : "";
    let own: ChatMessage | undefined;
    if (!thread || action === "edit" || action === "delete") {
      own = (await this.read(conversation, data.kind, typeof data.topics === "number" ? data.topics : 100)).messages.find(m => m.id === messageId);
      if (!own) throw new GoogleChatError("message not found in recent topics", 404);
      thread = own.thread;
    }
    const message = { parent_id: { topic_id: { group_id: g, topic_id: thread } }, message_id: messageId };
    if (action === "edit" || action === "delete") {
      if (own?.from_id !== await this.selfId()) throw new GoogleChatError("only own messages can be edited or deleted", 403);
      if (action === "delete") return this.rpc("delete_message", "DeleteMessageRequest", "DeleteMessageResponse", { message_id: message });
      if (typeof data.text !== "string" || !data.text || data.text.length > 32000) throw new GoogleChatError("invalid message text", 400);
      return this.rpc("edit_message", "EditMessageRequest", "EditMessageResponse", { message_id: message, text_body: data.text, message_info: messageInfo });
    }
    if (action === "react") {
      if (typeof data.emoji !== "string" || !data.emoji || data.emoji.length > 64) throw new GoogleChatError("invalid reaction", 400);
      return this.rpc("update_reaction", "UpdateReactionRequest", "UpdateReactionResponse", { message_id: message, emoji: { unicode: data.emoji }, type: data.remove === true ? "REMOVE" : "ADD" });
    }
    throw new GoogleChatError("unknown Chat action", 400);
  }
}
