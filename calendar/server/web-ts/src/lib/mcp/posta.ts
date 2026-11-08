import { config } from "../config";

export interface AccountToken {
  upn: string;
  token: string;
  exp: number;
  endpoint_type: "outlook_rest" | "graph";
  base_url: string;
  scopes: string;
  phone_status?: string;
  phone_msg?: string;
}

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "image"; data: string; mimeType: string }
  | { type: "resource"; resource: { uri: string; mimeType: string; text?: string; blob?: string } };

let cacheToken: { quandoMs: number; account: AccountToken[] } | null = null;
const CACHE_TOKEN_MS = 30_000;

export async function caricaAccountPosta(forza = false): Promise<AccountToken[]> {
  const adesso = Date.now();
  if (!forza && cacheToken && adesso - cacheToken.quandoMs < CACHE_TOKEN_MS && cacheToken.account.length > 0) {
    return cacheToken.account;
  }
  const res = await fetch(`${config.agenteUrl}/api/token-posta`, {
    headers: { Authorization: `Bearer ${config.agenteChiave}`, Accept: "application/json" },
    cache: "no-store",
  });
  if (!res.ok) {
    throw new Error(`Agente non raggiungibile per i token della posta (HTTP ${res.status})`);
  }
  const dati = (await res.json()) as { account?: AccountToken[] };
  const adessoS = adesso / 1000;
  const validi = (dati.account ?? []).filter((a) => a && typeof a.upn === "string" && typeof a.token === "string" && a.exp > adessoS);
  cacheToken = { quandoMs: adesso, account: validi };
  return validi;
}

export async function scegliAccount(filtro?: string, forza = false): Promise<AccountToken> {
  let lista = await caricaAccountPosta(forza);
  if (lista.length === 0 && !forza) lista = await caricaAccountPosta(true);
  if (lista.length === 0) {
    throw new Error("Nessuna casella Outlook con token valido trovata sul telefono di OE Copilot.");
  }
  if (!filtro || !filtro.trim()) return lista[0]!;
  const f = filtro.trim().toLowerCase();
  const trovato = lista.find((a) => a.upn.toLowerCase().includes(f));
  if (trovato) return trovato;
  if (!forza) {
    lista = await caricaAccountPosta(true);
    const ritentato = lista.find((a) => a.upn.toLowerCase().includes(f));
    if (ritentato) return ritentato;
  }
  throw new Error(`Casella '${filtro}' non trovata tra quelle attive sul telefono: ${lista.map((a) => a.upn).join(", ")}`);
}

async function richiestaOutlook(
  accountFiltro: string | undefined,
  percorso: string,
  metodo = "GET",
  dati?: unknown,
  riprova = true,
): Promise<{ info: AccountToken; json: Record<string, unknown> }> {
  const info = await scegliAccount(accountFiltro, !riprova);
  const url = `${info.base_url.replace(/\/+$/, "")}/${percorso.replace(/^\/+/, "")}`;
  const headers: Record<string, string> = {
    Authorization: `Bearer ${info.token}`,
    Accept: "application/json",
    "User-Agent": "OECopilot-MailMCP/1.0",
  };
  let body: string | undefined;
  if (dati !== undefined) {
    headers["Content-Type"] = "application/json; charset=utf-8";
    body = JSON.stringify(dati);
  }
  const res = await fetch(url, { method: metodo, headers, body, cache: "no-store" });
  if (res.status === 401 && riprova) {
    await caricaAccountPosta(true);
    return richiestaOutlook(accountFiltro, percorso, metodo, dati, false);
  }
  if (!res.ok) {
    const errText = await res.text().catch(() => "");
    throw new Error(`Errore API Outlook ${res.status} su ${metodo} ${percorso}: ${errText || res.statusText}`);
  }
  if (res.status === 204) return { info, json: {} };
  const testo = await res.text();
  return { info, json: testo ? (JSON.parse(testo) as Record<string, unknown>) : {} };
}

// ---------------------------------------------------------------- HTML <-> Markdown / Rich Body

const ENTITA_BASE: Record<string, string> = {
  "&nbsp;": " ",
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
  "&apos;": "'",
  "&ndash;": "-",
  "&mdash;": "-",
  "&euro;": "€",
  "&rsquo;": "'",
  "&lsquo;": "'",
  "&ldquo;": '"',
  "&rdquo;": '"',
  "&hellip;": "...",
};

function decodificaEntitaHtml(testo: string): string {
  return testo
    .replace(/&#(\d+);/g, (_, dec) => {
      const code = Number(dec);
      return Number.isFinite(code) && code > 0 ? String.fromCodePoint(code) : _;
    })
    .replace(/&#x([0-9a-fA-F]+);/g, (_, hex) => {
      const code = Number.parseInt(hex, 16);
      return Number.isFinite(code) && code > 0 ? String.fromCodePoint(code) : _;
    })
    .replace(/&[a-zA-Z0-9]+;/g, (m) => ENTITA_BASE[m.toLowerCase()] ?? m);
}

function pulisciSafeLink(url: string): string {
  try {
    const u = new URL(url);
    if (u.hostname.toLowerCase().includes("safelinks.protection.outlook.com")) {
      const target = u.searchParams.get("url");
      if (target) return target;
    }
  } catch {
    // Non è un URL assoluto valido
  }
  return url;
}

export function htmlInMarkdown(rawHtml: string): string {
  if (!rawHtml) return "";
  let s = rawHtml;
  s = s.replace(/<(style|script|head|title|xml)\b[^>]*>[\s\S]*?<\/\1>/gi, "");
  s = s.replace(/<!\[if[^\]]*\]>[\s\S]*?<!\[endif\]>/gi, "");
  s = s.replace(/<!--[\s\S]*?-->/g, "");

  for (let lv = 1; lv <= 6; lv++) {
    const hashes = "#".repeat(lv);
    const re = new RegExp(`<h${lv}\\b[^>]*>([\\s\\S]*?)<\\/h${lv}>`, "gi");
    s = s.replace(re, (_, inner: string) => `\n\n${hashes} ${inner.replace(/<[^>]+>/g, "").trim()}\n\n`);
  }

  s = s.replace(/<pre\b[^>]*>([\s\S]*?)<\/pre>/gi, (_, inner: string) => {
    return `\n\`\`\`\n${decodificaEntitaHtml(inner.replace(/<[^>]+>/g, "").trim())}\n\`\`\`\n`;
  });
  s = s.replace(/<code\b[^>]*>([\s\S]*?)<\/code>/gi, (_, inner: string) => {
    return `\`${decodificaEntitaHtml(inner.replace(/<[^>]+>/g, "").trim())}\``;
  });

  s = s.replace(/<(b|strong)(?:\s[^>]*)?>([\s\S]*?)<\/\1>/gi, (_, _tag, inner: string) => {
    const t = inner.trim();
    return t ? `**${t}**` : "";
  });
  s = s.replace(/<(i|em)(?:\s[^>]*)?>([\s\S]*?)<\/\1>/gi, (_, _tag, inner: string) => {
    const t = inner.trim();
    return t ? `*${t}*` : "";
  });

  s = s.replace(/<a\s+[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi, (_, href: string, inner: string) => {
    const url = pulisciSafeLink(decodificaEntitaHtml(href.trim()));
    const txt = pulisciSafeLink(decodificaEntitaHtml(inner.replace(/<[^>]+>/g, "").trim()));
    if (!txt || txt === url) return url;
    return `[${txt}](${url})`;
  });

  s = s.replace(/<table\b[^>]*>([\s\S]*?)<\/table>/gi, (_, tableInner: string) => {
    const trMatches = [...tableInner.matchAll(/<tr\b[^>]*>([\s\S]*?)<\/tr>/gi)];
    if (trMatches.length === 0) return _;
    const mdRows: string[] = [];
    for (const tr of trMatches) {
      const cells = [...(tr[1] ?? "").matchAll(/<(?:td|th)\b[^>]*>([\s\S]*?)<\/(?:td|th)>/gi)].map((c) =>
        decodificaEntitaHtml((c[1] ?? "").replace(/<br\s*\/?>/gi, " ").replace(/<[^>]+>/g, " "))
          .replace(/\u00a0/g, " ")
          .replace(/\s+/g, " ")
          .trim()
          .replace(/\|/g, "\\|"),
      );
      if (cells.length > 0 && cells.some((x) => x.length > 0)) {
        mdRows.push(`| ${cells.join(" | ")} |`);
        if (mdRows.length === 1) mdRows.push(`| ${cells.map(() => "---").join(" | ")} |`);
      }
    }
    return mdRows.length > 0 ? `\n\n${mdRows.join("\n")}\n\n` : "";
  });

  s = s.replace(/<blockquote\b[^>]*>([\s\S]*?)<\/blockquote>/gi, (_, inner: string) => {
    const q = htmlInMarkdown(inner);
    return (
      "\n" +
      q
        .split("\n")
        .filter((l) => l.trim())
        .map((l) => `> ${l}`)
        .join("\n") +
      "\n"
    );
  });

  s = s.replace(/<li\b[^>]*>/gi, "\n* ");
  s = s.replace(/<\/li>/gi, "");
  s = s.replace(/<br\s*\/?>/gi, "\n");
  s = s.replace(/<\/(p|div|tr|ul|ol|table)>/gi, "\n\n");
  s = s.replace(/<hr\b[^>]*/gi, "\n---\n");
  s = s.replace(/<[^>]+>/g, "");
  s = decodificaEntitaHtml(s);
  s = s.replace(/\u00a0/g, " ").replace(/\u200b/g, "").replace(/\r\n/g, "\n");
  s = s.replace(/[ \t]+/g, " ");
  s = s.replace(/ *\n */g, "\n");
  s = s.replace(/\n{3,}/g, "\n\n");
  return s.trim();
}

const TAG_HTML_RE =
  /<\s*(?:p|div|span|b|strong|i|em|u|mark|br|hr|ul|ol|li|table|thead|tbody|tr|th|td|h[1-6]|a|blockquote|pre|code|font|img)\b/i;

export function sembraHtml(testo: string): boolean {
  return Boolean(testo && TAG_HTML_RE.test(testo));
}

function escapeHtml(testo: string): string {
  return testo.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
}

export function formattaCorpoPerHtml(corpo: string, isHtml = false): string {
  if (!corpo) return "";
  if (isHtml || sembraHtml(corpo)) return corpo;
  const esc = escapeHtml(corpo).replace(/\r\n/g, "\n").replace(/\n/g, "<br>\n");
  return `<div style="font-family:Aptos,Calibri,sans-serif;font-size:11pt;">${esc}</div>`;
}

// ---------------------------------------------------------------- Destinatari & Formattazione Messaggi

interface IndirizzoEmail {
  EmailAddress: { Address: string; Name?: string };
}

export function analizzaDestinatari(val: unknown): IndirizzoEmail[] {
  if (!val) return [];
  const parti: unknown[] = [];
  if (typeof val === "string") {
    for (const p of val.split(/[,;]/)) if (p.trim()) parti.push(p.trim());
  } else if (Array.isArray(val)) {
    for (const item of val) {
      if (typeof item === "string") {
        for (const p of item.split(/[,;]/)) if (p.trim()) parti.push(p.trim());
      } else {
        parti.push(item);
      }
    }
  } else {
    parti.push(String(val));
  }

  const out: IndirizzoEmail[] = [];
  const visti = new Set<string>();
  for (const p of parti) {
    let voce: IndirizzoEmail | null = null;
    if (p && typeof p === "object") {
      const o = p as Record<string, unknown>;
      if (o.EmailAddress && typeof o.EmailAddress === "object") {
        const ea = o.EmailAddress as Record<string, unknown>;
        const addr = String(ea.Address ?? ea.address ?? "").trim();
        const name = String(ea.Name ?? ea.name ?? "").trim();
        if (addr) voce = { EmailAddress: name ? { Address: addr, Name: name } : { Address: addr } };
      } else {
        const addr = String(o.Address ?? o.address ?? "").trim();
        const name = String(o.Name ?? o.name ?? "").trim();
        if (addr) voce = { EmailAddress: name ? { Address: addr, Name: name } : { Address: addr } };
      }
    } else if (typeof p === "string") {
      const m = /<([^>]+)>/.exec(p);
      if (m?.[1]) {
        const addr = m[1].trim();
        const name = p.slice(0, m.index).trim().replace(/^["']|["']$/g, "");
        if (addr) voce = { EmailAddress: name ? { Address: addr, Name: name } : { Address: addr } };
      } else if (p.trim()) {
        voce = { EmailAddress: { Address: p.trim() } };
      }
    }
    if (voce) {
      const k = voce.EmailAddress.Address.toLowerCase();
      if (k && !visti.has(k)) {
        visti.add(k);
        out.push(voce);
      }
    }
  }
  return out;
}

function campo(obj: unknown, nome: string): unknown {
  if (!obj || typeof obj !== "object") return undefined;
  const rec = obj as Record<string, unknown>;
  if (nome in rec) return rec[nome];
  const cam = nome.charAt(0).toLowerCase() + nome.slice(1);
  if (cam in rec) return rec[cam];
  const low = nome.toLowerCase();
  for (const [k, v] of Object.entries(rec)) {
    if (k.toLowerCase() === low) return v;
  }
  return undefined;
}

function formattaData(iso: unknown): string {
  if (typeof iso !== "string" || !iso) return "-";
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso.slice(0, 16).replace("T", " ");
    const fmt = new Intl.DateTimeFormat("sv-SE", {
      timeZone: "Europe/Rome",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
    return fmt.format(d);
  } catch {
    return iso.slice(0, 16).replace("T", " ");
  }
}

function stringaDestinatario(rec: unknown): string {
  if (rec && typeof rec === "object") {
    const ea = (campo(rec, "EmailAddress") ?? rec) as Record<string, unknown>;
    const name = String(campo(ea, "Name") ?? "").trim();
    const addr = String(campo(ea, "Address") ?? "").trim();
    if (name && addr && name !== addr) return `${name} <${addr}>`;
    return addr || name || "?";
  }
  return String(rec ?? "?");
}

function listaDestinatariStr(recs: unknown): string {
  if (!Array.isArray(recs) || recs.length === 0) return "-";
  return recs.map(stringaDestinatario).join(", ");
}

function formattaRiepilogoMessaggio(m: Record<string, unknown>): string {
  const mid = String(campo(m, "Id") ?? "");
  const cid = String(campo(m, "ConversationId") ?? "");
  const dtStr = formattaData(campo(m, "ReceivedDateTime") ?? campo(m, "CreatedDateTime"));
  const sender = stringaDestinatario(campo(m, "From") ?? campo(m, "Sender"));
  const toRecs = campo(m, "ToRecipients");
  const ccRecs = campo(m, "CcRecipients");
  const subj = String(campo(m, "Subject") || "(No Subject)");
  const prev = String(campo(m, "BodyPreview") ?? "")
    .replace(/\r|\n/g, " ")
    .trim();
  const unread = campo(m, "IsRead") === false ? " [UNREAD]" : "";
  const draft = campo(m, "IsDraft") === true ? " [DRAFT]" : "";
  const att = campo(m, "HasAttachments") ? " [📎]" : "";
  const flagObj = campo(m, "Flag");
  const flagged = String(campo(flagObj, "FlagStatus") ?? "").toLowerCase() === "flagged" ? " [🚩]" : "";
  const imp = String(campo(m, "Importance") ?? "").toLowerCase() === "high" ? " [❗HIGH]" : "";

  const righe = [`[${dtStr}]${unread}${draft}${att}${flagged}${imp} From: ${sender}`];
  if (Array.isArray(toRecs) && toRecs.length > 0) righe.push(`To: ${listaDestinatariStr(toRecs)}`);
  if (Array.isArray(ccRecs) && ccRecs.length > 0) righe.push(`Cc: ${listaDestinatariStr(ccRecs)}`);
  righe.push(
    `Subject: ${subj}`,
    prev.length > 140 ? `Preview: ${prev.slice(0, 140)}...` : `Preview: ${prev}`,
    `Id: ${mid}`,
    `ThreadId: ${cid}`,
  );
  return righe.join("\n");
}

function formattaMessaggioCompleto(m: Record<string, unknown>): string {
  const mid = String(campo(m, "Id") ?? "");
  const cid = String(campo(m, "ConversationId") ?? "");
  const dtStr = formattaData(campo(m, "ReceivedDateTime") ?? campo(m, "SentDateTime") ?? campo(m, "CreatedDateTime"));
  const sender = stringaDestinatario(campo(m, "From") ?? campo(m, "Sender"));
  const toStr = listaDestinatariStr(campo(m, "ToRecipients"));
  const ccRecs = campo(m, "CcRecipients");
  const bccRecs = campo(m, "BccRecipients");
  const replyTo = campo(m, "ReplyTo");
  const subj = String(campo(m, "Subject") || "(No Subject)");
  const isDraft = campo(m, "IsDraft") === true;
  const isRead = campo(m, "IsRead");
  const imp = String(campo(m, "Importance") || "Normal");

  const bodyObj = campo(m, "Body");
  const rawBody = String(campo(bodyObj, "Content") ?? campo(m, "BodyPreview") ?? "");
  const bodyType = String(campo(bodyObj, "ContentType") || "HTML").toUpperCase();
  const bodyMd = bodyType === "HTML" ? htmlInMarkdown(rawBody) : rawBody.trim();

  const tags: string[] = [];
  if (isDraft) tags.push("DRAFT");
  if (isRead === false) tags.push("UNREAD");
  if (imp.toLowerCase() !== "normal") tags.push(`Importance: ${imp}`);

  const header = ["==================================================", `From: ${sender}`, `To: ${toStr}`];
  if (Array.isArray(ccRecs) && ccRecs.length > 0) header.push(`Cc: ${listaDestinatariStr(ccRecs)}`);
  if (Array.isArray(bccRecs) && bccRecs.length > 0) header.push(`Bcc: ${listaDestinatariStr(bccRecs)}`);
  if (Array.isArray(replyTo) && replyTo.length > 0) header.push(`Reply-To: ${listaDestinatariStr(replyTo)}`);
  header.push(`Date: ${dtStr}`);
  if (tags.length > 0) header.push(`Status: ${tags.join(", ")}`);
  header.push(`Subject: ${subj}`, `Message ID: ${mid}`, `Thread ID: ${cid}`);

  const atts = m.AttachmentsList;
  if (Array.isArray(atts) && atts.length > 0) {
    header.push("Attachments:");
    atts.forEach((a, idx) => {
      const sizeBytes = Number(campo(a, "Size") ?? 0);
      const sizeKb = sizeBytes > 0 ? Math.max(1, Math.trunc(sizeBytes / 1024)) : 0;
      const inlineTag = campo(a, "IsInline") ? ", inline" : "";
      const aid = String(campo(a, "Id") ?? "");
      header.push(`  #${idx + 1} 📎 ${String(campo(a, "Name") ?? "attachment")} (${sizeKb} KB, ${String(campo(a, "ContentType") ?? "")}${inlineTag}) [attachment_id: ${aid}]`);
    });
  }
  header.push("--------------------------------------------------");
  return `${header.join("\n")}\n\n${bodyMd}`;
}

// ---------------------------------------------------------------- Cartelle e Filtri

const CARTELLE_NOTE: Record<string, string> = {
  inbox: "Inbox",
  "posta in arrivo": "Inbox",
  sent: "SentItems",
  sentitems: "SentItems",
  "sent items": "SentItems",
  "posta inviata": "SentItems",
  drafts: "Drafts",
  bozze: "Drafts",
  archive: "Archive",
  archivio: "Archive",
  deleted: "DeletedItems",
  deleteditems: "DeletedItems",
  "deleted items": "DeletedItems",
  trash: "DeletedItems",
  "posta eliminata": "DeletedItems",
  junk: "JunkEmail",
  junkemail: "JunkEmail",
  spam: "JunkEmail",
  "posta indesiderata": "JunkEmail",
  outbox: "Outbox",
  "posta in uscita": "Outbox",
};

interface InfoCartella {
  id: string;
  name: string;
  displayName: string;
  totalCount: number;
  unreadCount: number;
}

async function elencaCartelle(account?: string, includiFiglie = true): Promise<{ info: AccountToken; cartelle: InfoCartella[] }> {
  const { info, json } = await richiestaOutlook(account, "me/MailFolders?$top=100");
  const valore = Array.isArray(json.value) ? (json.value as Record<string, unknown>[]) : [];
  const cartelle: InfoCartella[] = [];
  for (const f of valore) {
    const fid = String(campo(f, "Id") ?? "");
    const fname = String(campo(f, "DisplayName") ?? "");
    const childCount = Number(campo(f, "ChildFolderCount") ?? 0);
    cartelle.push({
      id: fid,
      name: fname,
      displayName: fname,
      totalCount: Number(campo(f, "TotalItemCount") ?? 0),
      unreadCount: Number(campo(f, "UnreadItemCount") ?? 0),
    });
    if (includiFiglie && childCount > 0 && fid) {
      try {
        const sub = await richiestaOutlook(account, `me/MailFolders/${encodeURIComponent(fid)}/childfolders?$top=100`);
        const subVal = Array.isArray(sub.json.value) ? (sub.json.value as Record<string, unknown>[]) : [];
        for (const cf of subVal) {
          const cname = String(campo(cf, "DisplayName") ?? "");
          cartelle.push({
            id: String(campo(cf, "Id") ?? ""),
            name: `${fname}/${cname}`,
            displayName: cname,
            totalCount: Number(campo(cf, "TotalItemCount") ?? 0),
            unreadCount: Number(campo(cf, "UnreadItemCount") ?? 0),
          });
        }
      } catch {
        // Ignora errori sulle sottocartelle
      }
    }
  }
  return { info, cartelle };
}

async function risolviIdCartella(account: string | undefined, cartella: string): Promise<string | null> {
  if (!cartella || !cartella.trim()) return "Inbox";
  const c = cartella.trim();
  const nota = CARTELLE_NOTE[c.toLowerCase()];
  if (nota) return nota;
  if (c.startsWith("AAMk") || c.startsWith("AQMk")) return c;
  const { cartelle } = await elencaCartelle(account, true);
  const low = c.toLowerCase();
  const esatta = cartelle.find((f) => f.name.toLowerCase() === low || f.displayName.toLowerCase() === low);
  if (esatta) return esatta.id;
  const parziale = cartelle.find((f) => f.name.toLowerCase().includes(low));
  return parziale ? parziale.id : null;
}

function normalizzaDataIso(d: string | undefined, fineGiorno = false): string | null {
  if (!d || !d.trim()) return null;
  const s = d.trim();
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return fineGiorno ? `${s}T23:59:59Z` : `${s}T00:00:00Z`;
  if (!s.endsWith("Z") && !/[+-]\d{2}:?\d{2}$/.test(s.slice(10))) return `${s}Z`;
  return s;
}

// ---------------------------------------------------------------- Implementazione Tool MCP

export async function toolListAccounts(): Promise<string> {
  const lista = await caricaAccountPosta();
  if (lista.length === 0) {
    return "Nessuna casella Outlook trovata sul telefono di OE Copilot.";
  }
  const adessoS = Date.now() / 1000;
  const righe = [`Found ${lista.length} active Outlook account(s) on OE Copilot:`, ""];
  for (const a of lista) {
    const expMin = Math.max(0, Math.trunc((a.exp - adessoS) / 60));
    const st = a.phone_status ? `, phone sync: ${a.phone_status}` : "";
    const msg = a.phone_msg ? ` (${a.phone_msg})` : "";
    righe.push(`- ${a.upn} [oe-copilot, ${a.endpoint_type}, token expires in ${expMin} min${st}]${msg}`);
  }
  return righe.join("\n");
}

export async function toolListFolders(args: Record<string, unknown>): Promise<string> {
  const account = typeof args.account === "string" ? args.account : undefined;
  const includiFiglie = args.include_child_folders !== false;
  const { info, cartelle } = await elencaCartelle(account, includiFiglie);
  const righe = [`Folders for ${info.upn}:`, ""];
  for (const f of cartelle) {
    const unread = f.unreadCount > 0 ? ` (${f.unreadCount} unread)` : "";
    righe.push(`- ${f.name} [${f.totalCount} items${unread}] (id: ${f.id})`);
  }
  return righe.join("\n");
}

export async function cercaOElencaMessaggi(opzioni: {
  account?: string;
  folder?: string;
  query?: string;
  fromAddress?: string;
  unreadOnly?: boolean;
  hasAttachments?: boolean;
  flaggedOnly?: boolean;
  startDate?: string;
  endDate?: string;
  limit?: number;
}): Promise<{ info: AccountToken; messaggi: Record<string, unknown>[] }> {
  const limit = Math.max(1, Math.min(Number(opzioni.limit || 20), 100));
  const folderRaw = opzioni.folder?.trim() || "Inbox";
  let basePath: string;
  if (["all", "*", "any"].includes(folderRaw.toLowerCase())) {
    basePath = "me/messages";
  } else {
    const fid = await risolviIdCartella(opzioni.account, folderRaw);
    basePath = fid ? `me/MailFolders/${encodeURIComponent(fid)}/messages` : "me/messages";
  }

  const serveFiltroLocale = Boolean(
    opzioni.fromAddress ||
      opzioni.flaggedOnly ||
      (opzioni.query && (opzioni.unreadOnly || opzioni.hasAttachments !== undefined || opzioni.startDate || opzioni.endDate)),
  );
  const fetchTop = serveFiltroLocale ? Math.min(Math.max(limit * 4, 50), 100) : limit;
  const selectFields =
    "Id,Subject,From,Sender,ToRecipients,CcRecipients,ReceivedDateTime,CreatedDateTime," +
    "BodyPreview,HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId,ParentFolderId";
  const params = [`$top=${fetchTop}`, `$select=${selectFields}`];

  const startIso = normalizzaDataIso(opzioni.startDate, false);
  const endIso = normalizzaDataIso(opzioni.endDate, true);

  if (opzioni.query) {
    params.push(`$search=${encodeURIComponent(`"${opzioni.query}"`)}`);
  } else {
    const filtri: string[] = [];
    if (opzioni.unreadOnly) filtri.push("IsRead eq false");
    if (opzioni.hasAttachments === true) filtri.push("HasAttachments eq true");
    else if (opzioni.hasAttachments === false) filtri.push("HasAttachments eq false");
    if (startIso) filtri.push(`ReceivedDateTime ge ${startIso}`);
    if (endIso) filtri.push(`ReceivedDateTime le ${endIso}`);
    if (filtri.length > 0) params.push(`$filter=${encodeURIComponent(filtri.join(" and "))}`);
    params.push(`$orderby=${encodeURIComponent("ReceivedDateTime desc")}`);
  }

  const { info, json } = await richiestaOutlook(opzioni.account, `${basePath}?${params.join("&")}`);
  let items = Array.isArray(json.value) ? (json.value as Record<string, unknown>[]) : [];

  if (serveFiltroLocale) {
    items = items.filter((m) => {
      if (opzioni.unreadOnly && campo(m, "IsRead") !== false) return false;
      if (opzioni.hasAttachments === true && !campo(m, "HasAttachments")) return false;
      if (opzioni.hasAttachments === false && campo(m, "HasAttachments")) return false;
      if (opzioni.flaggedOnly) {
        const flagSt = String(campo(campo(m, "Flag"), "FlagStatus") ?? "").toLowerCase();
        if (flagSt !== "flagged") return false;
      }
      if (opzioni.fromAddress) {
        const needle = opzioni.fromAddress.toLowerCase();
        const senderStr = stringaDestinatario(campo(m, "From") ?? campo(m, "Sender")).toLowerCase();
        if (!senderStr.includes(needle)) return false;
      }
      const msgDt = String(campo(m, "ReceivedDateTime") ?? campo(m, "CreatedDateTime") ?? "");
      if (startIso && msgDt && msgDt < startIso) return false;
      if (endIso && msgDt && msgDt > endIso) return false;
      return true;
    });
  }

  return { info, messaggi: items.slice(0, limit) };
}

export async function toolListEmails(args: Record<string, unknown>): Promise<string> {
  const folder = typeof args.folder === "string" && args.folder.trim() ? args.folder.trim() : "Inbox";
  const { info, messaggi } = await cercaOElencaMessaggi({
    account: typeof args.account === "string" ? args.account : undefined,
    folder,
    query: typeof args.query === "string" ? args.query : undefined,
    fromAddress: typeof args.from_address === "string" ? args.from_address : undefined,
    unreadOnly: Boolean(args.unread_only),
    hasAttachments: typeof args.has_attachments === "boolean" ? args.has_attachments : undefined,
    flaggedOnly: Boolean(args.flagged_only),
    startDate: typeof args.start_date === "string" ? args.start_date : undefined,
    endDate: typeof args.end_date === "string" ? args.end_date : undefined,
    limit: typeof args.limit === "number" ? args.limit : 20,
  });
  if (messaggi.length === 0) {
    return `No emails found in '${folder}' for ${info.upn} matching criteria.`;
  }
  return [
    `Account: ${info.upn} | Folder: ${folder} | Showing ${messaggi.length} message(s) (newest first)`,
    "",
    messaggi.map(formattaRiepilogoMessaggio).join("\n\n"),
  ].join("\n");
}

export async function toolSearchEmails(args: Record<string, unknown>): Promise<string> {
  const query = typeof args.query === "string" ? args.query.trim() : "";
  const fromAddress = typeof args.from_address === "string" ? args.from_address.trim() : "";
  if (!query && !fromAddress) return "query or from_address is required";
  const { info, messaggi } = await cercaOElencaMessaggi({
    account: typeof args.account === "string" ? args.account : undefined,
    folder: typeof args.folder === "string" && args.folder.trim() ? args.folder.trim() : "all",
    query: query || undefined,
    fromAddress: fromAddress || undefined,
    unreadOnly: Boolean(args.unread_only),
    hasAttachments: typeof args.has_attachments === "boolean" ? args.has_attachments : undefined,
    startDate: typeof args.start_date === "string" ? args.start_date : undefined,
    endDate: typeof args.end_date === "string" ? args.end_date : undefined,
    limit: typeof args.limit === "number" ? args.limit : 20,
  });
  const desc = query || `from:${fromAddress}`;
  if (messaggi.length === 0) return `No emails matching '${desc}' for ${info.upn}.`;
  return [
    `Search results for '${desc}' (${messaggi.length} hit(s), account: ${info.upn}):`,
    "",
    messaggi.map(formattaRiepilogoMessaggio).join("\n\n"),
  ].join("\n");
}

export async function toolReadEmail(args: Record<string, unknown>): Promise<string> {
  const account = typeof args.account === "string" ? args.account : undefined;
  const messageId = typeof args.message_id === "string" ? args.message_id.trim() : "";
  if (!messageId) return "message_id is required";
  const path = `me/messages/${encodeURIComponent(messageId)}`;
  const { json: msg } = await richiestaOutlook(account, path);
  if (campo(msg, "HasAttachments")) {
    try {
      const attRes = await richiestaOutlook(account, `${path}/attachments?$select=Id,Name,ContentType,Size,IsInline`);
      msg.AttachmentsList = Array.isArray(attRes.json.value) ? attRes.json.value : [];
    } catch {
      msg.AttachmentsList = [];
    }
  }
  return formattaMessaggioCompleto(msg);
}

export async function toolReadThread(args: Record<string, unknown>): Promise<string> {
  const account = typeof args.account === "string" ? args.account : undefined;
  const threadId = typeof args.thread_id === "string" ? args.thread_id.trim() : "";
  if (!threadId) return "thread_id (or message_id) is required";
  const limit = Math.max(1, Math.min(Number(args.limit || 50), 100));

  let cid = threadId;
  if (!threadId.startsWith("AAQk")) {
    try {
      const { json: m } = await richiestaOutlook(account, `me/messages/${encodeURIComponent(threadId)}?$select=ConversationId`);
      const trovato = campo(m, "ConversationId");
      if (typeof trovato === "string" && trovato) cid = trovato;
    } catch {
      // Usa threadId così com'è
    }
  }

  const filterExpr = encodeURIComponent(`ConversationId eq '${cid.replaceAll("'", "''")}'`);
  const selectFields =
    "Id,Subject,From,Sender,ToRecipients,CcRecipients,BccRecipients,ReplyTo," +
    "ReceivedDateTime,SentDateTime,CreatedDateTime,Body,BodyPreview," +
    "HasAttachments,IsRead,IsDraft,Importance,Flag,ConversationId";
  const { info, json } = await richiestaOutlook(account, `me/messages?$filter=${filterExpr}&$top=${limit}&$select=${selectFields}`);
  const messages = Array.isArray(json.value) ? (json.value as Record<string, unknown>[]) : [];
  if (messages.length === 0) return `No messages found for thread '${threadId}'`;

  messages.sort((a, b) => {
    const ta = String(campo(a, "ReceivedDateTime") ?? campo(a, "SentDateTime") ?? campo(a, "CreatedDateTime") ?? "");
    const tb = String(campo(b, "ReceivedDateTime") ?? campo(b, "SentDateTime") ?? campo(b, "CreatedDateTime") ?? "");
    return ta.localeCompare(tb);
  });

  for (const m of messages) {
    if (campo(m, "HasAttachments")) {
      try {
        const mid = String(campo(m, "Id") ?? "");
        const attRes = await richiestaOutlook(account, `me/messages/${encodeURIComponent(mid)}/attachments?$select=Id,Name,ContentType,Size,IsInline`);
        m.AttachmentsList = Array.isArray(attRes.json.value) ? attRes.json.value : [];
      } catch {
        m.AttachmentsList = [];
      }
    }
  }

  const partecipanti: string[] = [];
  const visti = new Set<string>();
  for (const m of messages) {
    const lista = [
      campo(m, "From") ?? campo(m, "Sender"),
      ...(Array.isArray(campo(m, "ToRecipients")) ? (campo(m, "ToRecipients") as unknown[]) : []),
      ...(Array.isArray(campo(m, "CcRecipients")) ? (campo(m, "CcRecipients") as unknown[]) : []),
    ];
    for (const r of lista) {
      if (!r) continue;
      const s = stringaDestinatario(r);
      const k = s.toLowerCase();
      if (s !== "?" && !visti.has(k)) {
        visti.add(k);
        partecipanti.push(s);
      }
    }
  }

  const subj = String(campo(messages[0], "Subject") || "(No Subject)");
  const out = [
    `Thread: ${subj} (${messages.length} message(s), oldest to newest)`,
    `Account: ${info.upn} | ConversationId: ${cid}`,
    `Participants: ${partecipanti.join(", ")}`,
    "",
  ];
  messages.forEach((m, i) => {
    out.push(`--- [Message ${i + 1} of ${messages.length}] ---`, formattaMessaggioCompleto(m), "");
  });
  return out.join("\n");
}

function unisciDestinatari(esistenti: unknown, extra: IndirizzoEmail[]): IndirizzoEmail[] {
  const base = analizzaDestinatari(esistenti);
  const visti = new Set(base.map((r) => r.EmailAddress.Address.toLowerCase()));
  for (const r of extra) {
    const k = r.EmailAddress.Address.toLowerCase();
    if (!visti.has(k)) {
      visti.add(k);
      base.push(r);
    }
  }
  return base;
}

function costruisciAllegatiInUscita(attachments: unknown): Record<string, unknown>[] {
  if (!Array.isArray(attachments)) return [];
  const out: Record<string, unknown>[] = [];
  for (const att of attachments) {
    if (att && typeof att === "object") {
      const o = att as Record<string, unknown>;
      const name = String(o.name ?? o.filename ?? o.Name ?? "attachment.bin").trim();
      let b64 = String(o.content_base64 ?? o.data ?? o.ContentBytes ?? "").trim();
      const m = /^data:([^;]+);base64,(.+)$/i.exec(b64);
      let ctype = typeof o.content_type === "string" ? o.content_type : undefined;
      if (m) {
        ctype = ctype ?? m[1];
        b64 = m[2] ?? "";
      }
      if (b64) {
        const voce: Record<string, unknown> = {
          "@odata.type": "#Microsoft.OutlookServices.FileAttachment",
          Name: name,
          ContentBytes: b64,
        };
        if (ctype) voce.ContentType = ctype;
        out.push(voce);
      }
    }
  }
  return out;
}

export async function toolSendEmail(args: Record<string, unknown>): Promise<string> {
  const account = typeof args.account === "string" ? args.account : undefined;
  const subject = typeof args.subject === "string" ? args.subject.trim() : "";
  const body = typeof args.body === "string" ? args.body : "";
  const isHtml = Boolean(args.is_html);
  const replyToId = typeof args.reply_to_id === "string" ? args.reply_to_id.trim() : "";
  const replyAll = Boolean(args.reply_all);
  const forwardToId = typeof args.forward_to_id === "string" ? args.forward_to_id.trim() : "";
  const replaceCc = Boolean(args.replace_cc);
  const saveAsDraft = Boolean(args.save_as_draft);
  const importance = typeof args.importance === "string" ? args.importance : undefined;

  const parsedTo = analizzaDestinatari(args.to);
  const parsedCc = analizzaDestinatari(args.cc);
  const parsedBcc = analizzaDestinatari(args.bcc);
  const esclusi = new Set(analizzaDestinatari(args.exclude_recipients).map((r) => r.EmailAddress.Address.toLowerCase()));
  const allegati = costruisciAllegatiInUscita(args.attachments);

  const bersaglioId = replyToId || forwardToId;
  if (bersaglioId) {
    if (replyToId && !body.trim() && !saveAsDraft) return "body comment is required when replying";
    if (forwardToId && parsedTo.length === 0 && !saveAsDraft) return "'to' recipient is required when forwarding an email";

    const azione = replyToId ? (replyAll ? "createreplyall" : "createreply") : "createforward";
    const commentoHtml = formattaCorpoPerHtml(body, isHtml);
    const { info, json: draft } = await richiestaOutlook(account, `me/messages/${encodeURIComponent(bersaglioId)}/${azione}`, "POST", {
      Comment: commentoHtml,
    });
    const draftId = String(campo(draft, "Id") ?? "");
    if (!draftId) throw new Error(`Impossibile creare la bozza (${azione})`);
    const draftPath = `me/messages/${encodeURIComponent(draftId)}`;

    let toRecs = parsedTo.length > 0 ? parsedTo : analizzaDestinatari(campo(draft, "ToRecipients"));
    let ccRecs =
      replaceCc || azione !== "createreplyall"
        ? parsedCc.length > 0
          ? parsedCc
          : replaceCc
            ? []
            : analizzaDestinatari(campo(draft, "CcRecipients"))
        : unisciDestinatari(campo(draft, "CcRecipients"), parsedCc);
    let bccRecs = unisciDestinatari(campo(draft, "BccRecipients"), parsedBcc);

    if (esclusi.size > 0) {
      toRecs = toRecs.filter((r) => !esclusi.has(r.EmailAddress.Address.toLowerCase()));
      ccRecs = ccRecs.filter((r) => !esclusi.has(r.EmailAddress.Address.toLowerCase()));
      bccRecs = bccRecs.filter((r) => !esclusi.has(r.EmailAddress.Address.toLowerCase()));
    }

    if (toRecs.length === 0 && !saveAsDraft) {
      await richiestaOutlook(account, draftPath, "DELETE").catch(() => undefined);
      throw new Error("Nessun destinatario 'To' rimasto: specifica esplicitamente il parametro 'to'.");
    }

    const patchData: Record<string, unknown> = {
      ToRecipients: toRecs,
      CcRecipients: ccRecs,
      BccRecipients: bccRecs,
    };
    if (subject) patchData.Subject = subject;
    if (importance && ["Low", "Normal", "High"].includes(importance)) patchData.Importance = importance;

    try {
      await richiestaOutlook(account, draftPath, "PATCH", patchData);
      for (const att of allegati) {
        await richiestaOutlook(account, `${draftPath}/attachments`, "POST", att);
      }
      if (!saveAsDraft) {
        await richiestaOutlook(account, `${draftPath}/send`, "POST");
      }
    } catch (e) {
      if (!saveAsDraft) await richiestaOutlook(account, draftPath, "DELETE").catch(() => undefined);
      throw e;
    }

    const etichetta = replyToId ? `Reply (${replyAll ? "reply-all" : "selective/single reply"})` : "Forward";
    const parti = [
      `${etichetta} ${saveAsDraft ? "saved as draft" : "successfully sent"} from ${info.upn}.`,
      `To: [${toRecs.map((r) => r.EmailAddress.Address).join(", ")}]`,
    ];
    if (ccRecs.length > 0) parti.push(`Cc: [${ccRecs.map((r) => r.EmailAddress.Address).join(", ")}]`);
    if (bccRecs.length > 0) parti.push(`Bcc: [${bccRecs.map((r) => r.EmailAddress.Address).join(", ")}]`);
    if (saveAsDraft) parti.push(`Draft ID: ${draftId}`);
    return parti.join(" | ");
  }

  if (parsedTo.length === 0 && !saveAsDraft) return "'to' recipient is required for sending new email";
  if (!body.trim() && !subject.trim()) return "Cannot send empty email (both subject and body are empty)";

  const usaHtml = isHtml || sembraHtml(body);
  const message: Record<string, unknown> = {
    Subject: subject,
    Body: {
      ContentType: usaHtml ? "HTML" : "Text",
      Content: body,
    },
    ToRecipients: parsedTo,
  };
  if (parsedCc.length > 0) message.CcRecipients = parsedCc;
  if (parsedBcc.length > 0) message.BccRecipients = parsedBcc;
  if (importance && ["Low", "Normal", "High"].includes(importance)) message.Importance = importance;

  if (saveAsDraft) {
    const { info, json: draft } = await richiestaOutlook(account, "me/messages", "POST", message);
    const draftId = String(campo(draft, "Id") ?? "");
    for (const att of allegati) {
      await richiestaOutlook(account, `me/messages/${encodeURIComponent(draftId)}/attachments`, "POST", att);
    }
    return `Email saved as draft from ${info.upn}. | Subject: ${subject} | To: [${parsedTo.map((r) => r.EmailAddress.Address).join(", ")}] | Draft ID: ${draftId}`;
  }

  if (allegati.length > 0) message.Attachments = allegati;
  const { info } = await richiestaOutlook(account, "me/sendmail", "POST", {
    Message: message,
    SaveToSentItems: "true",
  });
  const parti = [
    `Email '${subject}' successfully sent from ${info.upn}.`,
    `To: [${parsedTo.map((r) => r.EmailAddress.Address).join(", ")}]`,
  ];
  if (parsedCc.length > 0) parti.push(`Cc: [${parsedCc.map((r) => r.EmailAddress.Address).join(", ")}]`);
  if (parsedBcc.length > 0) parti.push(`Bcc: [${parsedBcc.map((r) => r.EmailAddress.Address).join(", ")}]`);
  return parti.join(" | ");
}

export async function toolForwardEmail(args: Record<string, unknown>): Promise<string> {
  const id = typeof args.message_id === "string" ? args.message_id : typeof args.forward_to_id === "string" ? args.forward_to_id : "";
  if (!id) return "message_id is required to forward an email";
  return toolSendEmail({ ...args, forward_to_id: id });
}

export async function toolGetAttachment(args: Record<string, unknown>): Promise<ContentBlock[]> {
  const account = typeof args.account === "string" ? args.account : undefined;
  const messageId = typeof args.message_id === "string" ? args.message_id.trim() : "";
  if (!messageId) return [{ type: "text", text: "message_id is required" }];

  const { info, json: listJson } = await richiestaOutlook(
    account,
    `me/messages/${encodeURIComponent(messageId)}/attachments?$select=Id,Name,ContentType,Size,IsInline`,
  );
  const atts = Array.isArray(listJson.value) ? (listJson.value as Record<string, unknown>[]) : [];
  if (atts.length === 0) return [{ type: "text", text: `No attachments on message ${messageId}.` }];

  const attId = typeof args.attachment_id === "string" ? args.attachment_id.trim() : "";
  const idxArg = typeof args.index === "number" ? Math.trunc(args.index) : undefined;

  let scelti: { idx: number; meta: Record<string, unknown> }[] = [];
  if (attId) {
    atts.forEach((a, i) => {
      if (String(campo(a, "Id") ?? "") === attId) scelti.push({ idx: i + 1, meta: a });
    });
    if (scelti.length === 0) {
      return [{ type: "text", text: `attachment_id not found on message ${messageId}; it has ${atts.length} attachment(s).` }];
    }
  } else if (idxArg !== undefined) {
    if (idxArg < 1 || idxArg > atts.length) {
      return [{ type: "text", text: `index ${idxArg} out of range: message ${messageId} has ${atts.length} attachment(s).` }];
    }
    scelti = [{ idx: idxArg, meta: atts[idxArg - 1]! }];
  } else {
    scelti = atts.map((meta, i) => ({ idx: i + 1, meta }));
  }

  const righe = [`${atts.length} attachment(s) on message ${messageId}:`];
  const blocchiExtra: ContentBlock[] = [];

  for (const { idx, meta } of scelti) {
    const aid = String(campo(meta, "Id") ?? "");
    const { json: full } = await richiestaOutlook(account, `me/messages/${encodeURIComponent(messageId)}/attachments/${encodeURIComponent(aid)}`);
    const otype = String(full["@odata.type"] ?? meta["@odata.type"] ?? "").toLowerCase();
    const name = String(campo(full, "Name") ?? campo(meta, "Name") ?? `attachment-${idx}`);
    const mime = String(campo(full, "ContentType") ?? campo(meta, "ContentType") ?? "application/octet-stream")
      .split(";")[0]!
      .trim();

    if (otype.includes("itemattachment")) {
      const expandParam =
        info.endpoint_type === "graph"
          ? "$expand=microsoft.graph.itemAttachment/item"
          : "$expand=Microsoft.OutlookServices.ItemAttachment/Item";
      try {
        const { json: exp } = await richiestaOutlook(
          account,
          `me/messages/${encodeURIComponent(messageId)}/attachments/${encodeURIComponent(aid)}?${expandParam}`,
        );
        const item = campo(exp, "Item") as Record<string, unknown> | undefined;
        if (item && typeof item === "object") {
          righe.push(`  #${idx} "${name}": embedded Outlook item:\n${formattaMessaggioCompleto(item)}`);
          continue;
        }
      } catch {
        // Fallback sotto
      }
      righe.push(`  #${idx} "${name}": embedded Outlook item.`);
      continue;
    }

    if (otype.includes("referenceattachment")) {
      const url = String(campo(full, "SourceUrl") ?? "");
      righe.push(`  #${idx} "${name}": cloud link (OneDrive/SharePoint): ${url}`.trimEnd());
      continue;
    }

    const contentB64 = String(campo(full, "ContentBytes") ?? "");
    if (!contentB64) {
      righe.push(`  #${idx} "${name}": no content bytes returned.`);
      continue;
    }
    const buf = Buffer.from(contentB64, "base64");
    righe.push(`  #${idx} "${name}" (${(buf.length / 1024).toFixed(1)} KB, ${mime})`);
    if (mime.startsWith("image/")) {
      blocchiExtra.push({ type: "image", data: contentB64, mimeType: mime });
    } else if (mime.startsWith("text/") || ["application/json", "application/xml", "application/csv"].includes(mime)) {
      blocchiExtra.push({
        type: "text",
        text: `--- Content of ${name} ---\n${buf.toString("utf8").slice(0, 100_000)}`,
      });
    } else if (buf.length <= 8 * 1024 * 1024) {
      blocchiExtra.push({
        type: "resource",
        resource: {
          uri: `attachment:///${encodeURIComponent(messageId)}/${encodeURIComponent(name)}`,
          mimeType: mime,
          blob: contentB64,
        },
      });
    }
  }

  return [{ type: "text", text: righe.join("\n") }, ...blocchiExtra];
}

export async function toolManageEmail(args: Record<string, unknown>): Promise<string> {
  const account = typeof args.account === "string" ? args.account : undefined;
  const rawIds = args.message_id;
  const ids = typeof rawIds === "string" ? [rawIds] : Array.isArray(rawIds) ? rawIds.map(String) : [];
  const action = typeof args.action === "string" ? args.action.trim().toLowerCase() : "";
  if (ids.length === 0 || !action) return "message_id and action are required";

  let destId: string | null = null;
  if (action === "archive") destId = "Archive";
  else if (action === "delete") destId = "DeletedItems";
  else if (action === "move") {
    const destFolder = typeof args.destination_folder === "string" ? args.destination_folder : "";
    if (!destFolder) return "destination_folder is required when action is 'move'";
    destId = await risolviIdCartella(account, destFolder);
    if (!destId) return `Destination folder '${destFolder}' not found`;
  } else if (!["mark_read", "mark_unread", "flag", "unflag"].includes(action)) {
    return `Unsupported action '${action}'. Use: mark_read, mark_unread, flag, unflag, archive, delete, move.`;
  }

  const info = await scegliAccount(account);
  const esiti: string[] = [`Executed '${action}' on ${ids.length} message(s) for ${info.upn}:`];
  for (const mid of ids) {
    const mpath = `me/messages/${encodeURIComponent(mid)}`;
    if (action === "mark_read") {
      await richiestaOutlook(account, mpath, "PATCH", { IsRead: true });
      esiti.push(`- ${mid.slice(0, 32)}...: marked_read`);
    } else if (action === "mark_unread") {
      await richiestaOutlook(account, mpath, "PATCH", { IsRead: false });
      esiti.push(`- ${mid.slice(0, 32)}...: marked_unread`);
    } else if (action === "flag") {
      await richiestaOutlook(account, mpath, "PATCH", { Flag: { FlagStatus: "Flagged" } });
      esiti.push(`- ${mid.slice(0, 32)}...: flagged`);
    } else if (action === "unflag") {
      await richiestaOutlook(account, mpath, "PATCH", { Flag: { FlagStatus: "NotFlagged" } });
      esiti.push(`- ${mid.slice(0, 32)}...: unflagged`);
    } else if (destId) {
      await richiestaOutlook(account, `${mpath}/move`, "POST", { DestinationId: destId });
      esiti.push(`- ${mid.slice(0, 32)}...: moved_to_${destId}`);
    }
  }
  return esiti.join("\n");
}

// ---------------------------------------------------------------- Catalogo Tool MCP

const ACCOUNT_PROP = {
  type: "string",
  description: "Filter or select by account email (e.g. part of address). If omitted, uses default account.",
};

export const MCP_TOOLS = [
  {
    name: "list_accounts",
    description: "List all signed-in Outlook accounts on the OE Copilot virtual phone with their token expiry and sync status.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "list_folders",
    description: "List mail folders (Inbox, SentItems, Drafts, Archive, DeletedItems, and custom/child folders) with unread and total message counts.",
    inputSchema: {
      type: "object",
      properties: {
        account: ACCOUNT_PROP,
        include_child_folders: { type: "boolean", description: "Include nested subfolders (default true)." },
      },
    },
  },
  {
    name: "list_emails",
    description:
      "List emails in a folder (default Inbox) sorted newest first. Returns summaries with message ID, thread ID, From, To, Cc, subject, and preview.",
    inputSchema: {
      type: "object",
      properties: {
        account: ACCOUNT_PROP,
        folder: { type: "string", description: 'Folder name (Inbox, SentItems, Archive, Drafts, DeletedItems, custom folder, or "all") or folder ID.' },
        query: { type: "string", description: "Optional search keyword to filter emails." },
        from_address: { type: "string", description: "Optional filter by sender email address or display name." },
        unread_only: { type: "boolean", description: "If true, only return unread emails." },
        has_attachments: { type: "boolean", description: "If true, only return emails with attachments." },
        flagged_only: { type: "boolean", description: "If true, only return flagged emails." },
        start_date: { type: "string", description: "Optional start date filter (YYYY-MM-DD)." },
        end_date: { type: "string", description: "Optional end date filter (YYYY-MM-DD)." },
        limit: { type: "integer", description: "Max emails to return (default 20)." },
      },
    },
  },
  {
    name: "read_email",
    description: "Read full content of a specific email by its message ID, including From, To, Cc, Bcc, clean Markdown body (with tables), and attachments list.",
    inputSchema: {
      type: "object",
      required: ["message_id"],
      properties: {
        message_id: { type: "string", description: "The message ID from list_emails or search_emails." },
        account: ACCOUNT_PROP,
      },
    },
  },
  {
    name: "read_thread",
    description:
      "Read an entire email conversation thread by its thread_id (ConversationId) or any message_id within the thread. Messages are returned oldest to newest with full participants list, Markdown body, and attachments.",
    inputSchema: {
      type: "object",
      required: ["thread_id"],
      properties: {
        thread_id: { type: "string", description: "ConversationId or any message ID from the thread." },
        account: ACCOUNT_PROP,
        limit: { type: "integer", description: "Max messages in thread to return (default 50)." },
      },
    },
  },
  {
    name: "search_emails",
    description: "Search emails across all folders (or a specific folder) by keyword, sender, date range, unread status, or attachment presence.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query string." },
        from_address: { type: "string", description: "Filter by sender email or name." },
        folder: { type: "string", description: "Optional folder name/ID to restrict search." },
        unread_only: { type: "boolean", description: "If true, only return unread emails." },
        has_attachments: { type: "boolean", description: "If true, only return emails with attachments." },
        start_date: { type: "string", description: "Optional start date filter (YYYY-MM-DD)." },
        end_date: { type: "string", description: "Optional end date filter (YYYY-MM-DD)." },
        account: ACCOUNT_PROP,
        limit: { type: "integer", description: "Max results (default 20)." },
      },
    },
  },
  {
    name: "send_email",
    description:
      "Send a new email, reply to an existing email/thread, forward an email, or save as draft. Supports rich HTML formatting (is_html=true or HTML tags) on both new emails and replies/forwards, multiple CC/BCC recipients, selective replies to specific people (to, cc, replace_cc, exclude_recipients), and attachments.",
    inputSchema: {
      type: "object",
      properties: {
        account: ACCOUNT_PROP,
        to: {
          description:
            "Recipient email address or list of addresses. Required for new/forward emails; on replies, overrides ToRecipients so you can reply only to specific people.",
          type: ["string", "array"],
        },
        subject: { type: "string", description: "Subject of the email." },
        body: { type: "string", description: "Text or rich HTML body content." },
        cc: { description: "CC recipient(s) as string or array of addresses.", type: ["string", "array"] },
        bcc: { description: "BCC recipient(s) as string or array of addresses.", type: ["string", "array"] },
        is_html: { type: "boolean", description: "True if body contains rich HTML formatting (works on new emails, replies, and forwards)." },
        reply_to_id: { type: "string", description: "Message ID to reply to." },
        reply_all: { type: "boolean", description: "If replying, true to reply-all, false to reply only to sender/specified recipients." },
        replace_cc: { type: "boolean", description: "When replying with reply_all=true, set true to replace the CC list with `cc` instead of merging." },
        exclude_recipients: { description: "Email address(es) to exclude from To/Cc when replying-all.", type: ["string", "array"] },
        forward_to_id: { type: "string", description: "Message ID to forward." },
        importance: { type: "string", enum: ["Low", "Normal", "High"], description: "Message importance." },
        attachments: {
          type: "array",
          description: "Attachments as objects { name, content_base64, content_type? }.",
          items: {
            type: "object",
            required: ["name", "content_base64"],
            properties: {
              name: { type: "string" },
              content_base64: { type: "string" },
              content_type: { type: "string" },
            },
          },
        },
        save_as_draft: { type: "boolean", description: "If true, saves in Drafts without sending." },
      },
    },
  },
  {
    name: "forward_email",
    description:
      "Forward an existing email (preserving its original server-side attachments and quoted thread) to one or more recipients, with optional rich HTML/text comment and CC/BCC.",
    inputSchema: {
      type: "object",
      required: ["message_id", "to"],
      properties: {
        message_id: { type: "string", description: "Message ID to forward." },
        to: { description: "Recipient email address or list of addresses.", type: ["string", "array"] },
        body: { type: "string", description: "Optional text or rich HTML comment added above the forwarded message." },
        cc: { description: "CC recipient(s).", type: ["string", "array"] },
        bcc: { description: "BCC recipient(s).", type: ["string", "array"] },
        subject: { type: "string", description: "Optional custom subject override." },
        is_html: { type: "boolean", description: "True if body is HTML." },
        save_as_draft: { type: "boolean", description: "If true, saves the forward as a draft without sending." },
        account: ACCOUNT_PROP,
      },
    },
  },
  {
    name: "get_attachment",
    description:
      "Fetch attachment(s) of an email: returns images inline, text files as text, binary files as embedded resources, and expands embedded Outlook emails (ItemAttachment) into Markdown.",
    inputSchema: {
      type: "object",
      required: ["message_id"],
      properties: {
        message_id: { type: "string", description: "The message ID holding the attachment(s)." },
        attachment_id: { type: "string", description: "Optional Attachment Id." },
        index: { type: "integer", description: "Optional 1-based attachment index." },
        account: ACCOUNT_PROP,
      },
    },
  },
  {
    name: "manage_email",
    description: "Mark email(s) as read/unread, flag/unflag, archive, delete (move to DeletedItems), or move to a specific folder.",
    inputSchema: {
      type: "object",
      required: ["message_id", "action"],
      properties: {
        message_id: { description: "Message ID or array of message IDs.", type: ["string", "array"] },
        action: {
          type: "string",
          enum: ["mark_read", "mark_unread", "flag", "unflag", "archive", "delete", "move"],
          description: "Action to perform.",
        },
        destination_folder: { type: "string", description: 'Target folder name or ID (required when action is "move").' },
        account: ACCOUNT_PROP,
      },
    },
  },
];

export async function eseguiToolMcp(nome: string, args: Record<string, unknown>): Promise<ContentBlock[]> {
  switch (nome) {
    case "list_accounts":
      return [{ type: "text", text: await toolListAccounts() }];
    case "list_folders":
      return [{ type: "text", text: await toolListFolders(args) }];
    case "list_emails":
      return [{ type: "text", text: await toolListEmails(args) }];
    case "read_email":
      return [{ type: "text", text: await toolReadEmail(args) }];
    case "read_thread":
      return [{ type: "text", text: await toolReadThread(args) }];
    case "search_emails":
      return [{ type: "text", text: await toolSearchEmails(args) }];
    case "send_email":
      return [{ type: "text", text: await toolSendEmail(args) }];
    case "forward_email":
      return [{ type: "text", text: await toolForwardEmail(args) }];
    case "get_attachment":
      return await toolGetAttachment(args);
    case "manage_email":
      return [{ type: "text", text: await toolManageEmail(args) }];
    default:
      throw new Error(`Unknown tool: ${nome}`);
  }
}
