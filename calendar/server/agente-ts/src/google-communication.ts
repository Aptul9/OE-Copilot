import path from "node:path";
import { GoogleChat, GoogleChatError } from "./gchat.ts";
import { GmailGateway } from "./gmail.ts";
import { ArchivioGoogleWeb, leggiSessioneImportata, type SessioneGoogleWeb } from "./google-web.ts";
import type { FetchFn } from "./outlook-rest.ts";

type Data = Record<string, unknown>;
const exactAccount = (value: unknown): string => {
  if (typeof value !== "string" || value.length > 320 || !/^[^\s@]+@[^\s@]+$/.test(value)) throw new GoogleChatError("exact account email required", 400);
  return value.toLowerCase();
};

export class GoogleCommunication {
  readonly gmail: GmailGateway;
  readonly sessions: ArchivioGoogleWeb;
  private readonly options: { fetchFn?: FetchFn; now?: () => number };
  private readonly clients = new Map<string, GoogleChat>();

  constructor(directory: string, options: { fetchFn?: FetchFn; now?: () => number } = {}) {
    this.options = options;
    this.gmail = new GmailGateway(path.join(directory, "gmail.json"), options);
    this.sessions = new ArchivioGoogleWeb(path.join(directory, "google-web.json"));
  }

  accounts(): Data[] {
    return [...this.gmail.accounts(), ...this.sessions.elenco().map(s => ({ account: s.account, provider: "gchat", self_id: s.self_id ?? "", status: s.errore ? "errore" : "ok", message: s.errore ? "Google Chat session import required" : "" }))];
  }

  async importChat(input: Data): Promise<{ account: string; provider: string }> {
    const account = exactAccount(input.account ?? input.email);
    const now = (this.options.now ?? Date.now)();
    const data = leggiSessioneImportata(input.sessione, Math.floor(now / 1000));
    const session: SessioneGoogleWeb = { ...data, account, salvata: Math.floor(now / 1000) };
    const client = new GoogleChat(session, this.options);
    const identity = await client.identity();
    session.self_id = identity.self_id;
    // Validate before replacing a working session. Failed import leaves stored credentials intact.
    this.sessions.metti(session);
    this.clients.delete(account);
    return { account, provider: "gchat" };
  }

  private client(value: unknown): GoogleChat {
    const account = exactAccount(value);
    const session = this.sessions.sessione(account);
    if (!session) throw new GoogleChatError("Google Chat account not found", 404);
    let client = this.clients.get(account);
    if (!client || client.session !== session) {
      client = new GoogleChat(session, { ...this.options, fetchFn: (input, init) => {
        if (this.sessions.sessione(account) !== session) throw new GoogleChatError("Google Chat account changed", 409);
        return (this.options.fetchFn ?? fetch)(input, init);
      }, changed: () => {
        // In-flight work from a removed/replaced session cannot reinsert that session.
        if (this.sessions.sessione(account) === session) this.sessions.salva();
      } });
      this.clients.set(account, client);
    }
    return client;
  }

  private async chatOperation(account: unknown, operation: (client: GoogleChat) => Promise<unknown>): Promise<unknown> {
    const client = this.client(account);
    try {
      const result = await operation(client);
      if (client.session.errore && this.sessions.sessione(client.session.account) === client.session) {
        delete client.session.errore;
        this.sessions.salva();
      }
      return result;
    } catch (error) {
      if (error instanceof GoogleChatError && error.status === 409 && this.sessions.sessione(client.session.account) === client.session) {
        client.session.errore = "Google Chat session import required";
        this.sessions.salva();
      }
      throw error;
    }
  }

  readChat(data: Data): Promise<unknown> {
    return this.chatOperation(data.account, async client => {
      if (data.topics !== undefined && typeof data.topics !== "number") throw new GoogleChatError("topics must be a number", 400);
      if (data.action === "list") return client.list();
      if (data.action === "search") {
        if (typeof data.query !== "string") throw new GoogleChatError("query required", 400);
        return client.search(data.query, typeof data.topics === "number" ? data.topics : 100);
      }
      if (data.action === "attachment") {
        if (typeof data.token !== "string" || typeof data.mime_type !== "string") throw new GoogleChatError("token and mime_type required", 400);
        return client.attachment(data.token, data.mime_type);
      }
      if (data.action === "read" || data.action === "group") {
        if (typeof data.conversation !== "string" || (data.kind !== "space" && data.kind !== "dm")) throw new GoogleChatError("conversation and kind required", 400);
        return data.action === "group" ? client.details(data.conversation, data.kind) : client.read(data.conversation, data.kind, typeof data.topics === "number" ? data.topics : 100);
      }
      throw new GoogleChatError("unknown Chat read action", 400);
    });
  }

  writeChat(data: Data): Promise<unknown> {
    return this.chatOperation(data.account, client => client.write(data));
  }

  remove(provider: string, value: unknown): boolean {
    const account = exactAccount(value);
    if (provider === "gmail") {
      const exists = this.gmail.accounts().some(a => a.account === account);
      if (exists) this.gmail.tokens.rimuovi(account);
      return exists;
    }
    this.clients.delete(account);
    return this.sessions.rimuovi(account);
  }
}
