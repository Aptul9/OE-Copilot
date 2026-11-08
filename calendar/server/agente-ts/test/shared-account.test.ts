import { allowed, renewShared } from '../src/shared-account.ts';
import { describe, it, expect } from 'vitest';
import { creaServer } from '../src/index.ts';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { GestoreToken } from '../src/diretto.ts';
const jwt = (aud: string, oid = 'object') => 'x.' + Buffer.from(JSON.stringify({ aud, oid, tid: 'tenant', exp: 9000, scp: "Mail.ReadWrite Calendars.ReadWrite" })).toString('base64url') + '.x';
function setup(fetchFn: typeof fetch) {
 const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'shared-'));
 const file = path.join(dir, 'tokens.json');
 const entry = { provider: 'microsoft', refresh_token: 'initial', shared: { tenantId: 'tenant', objectId: 'object', capabilities: ['mail', 'calendar', 'chat'], origin: 'mobile' } };
 fs.writeFileSync(file, JSON.stringify({ accounts: { user: entry } }));
 return { file, entry, manager: new GestoreToken(file, { fetchFn, adesso: () => 1000000 }), cleanup: () => fs.rmSync(dir, { recursive: true }) };
}
describe('shared broker', () => {
 it('serializes Outlook and Teams rotations, persists cache across restart', async () => {
 const seen: string[] = [];
 const fetchFn = (async (_url, init) => {
 const body = new URLSearchParams(String(init?.body)); seen.push(body.get('refresh_token')!);
 await new Promise(r => setTimeout(r, 5));
 return Response.json({ access_token: jwt(body.get('scope')!.split('/.default')[0]!), refresh_token: 'rotated' + seen.length });
 }) as typeof fetch;
 const s = setup(fetchFn);
 try { const [mail, chat] = await Promise.all([s.manager.ottieniAccessToken('user'), s.manager.ottieniRisorsa('user', 'csa')]);
 expect(mail).not.toBe(chat); expect(seen).toEqual(['initial', 'rotated1']);
 expect(await new GestoreToken(s.file, { fetchFn, adesso: () => 1000000 }).ottieniRisorsa('user', 'csa')).toBe(chat); expect(seen).toHaveLength(2);
 } finally { s.cleanup(); }
 });
 it.each(['wrong-audience', 'wrong-object'])('rejects %s without persisting rotation', async kind => {
 const s = setup((async () => Response.json({ access_token: jwt(kind === 'wrong-audience' ? 'https://evil.invalid' : 'https://outlook.office.com', kind === 'wrong-object' ? 'other' : 'object'), refresh_token: 'bad' })) as typeof fetch);
 try { await expect(s.manager.ottieniAccessToken('user')).rejects.toThrow('mismatch'); expect(JSON.parse(fs.readFileSync(s.file, 'utf8')).accounts.user.refresh_token).toBe('initial'); } finally { s.cleanup(); }
 });
 it.each(['remove', 'replace'])('rejects credential %s during renewal', async action => {
 let release!: () => void;
 const gate = new Promise<void>(r => { release = r; });
 const s = setup((async () => { await gate; return Response.json({ access_token: jwt('https://outlook.office.com'), refresh_token: 'stale' }); }) as typeof fetch);
 try { const pending = s.manager.ottieniAccessToken('user');
 if (action === 'remove') s.manager.rimuovi('user');
 else { fs.writeFileSync(s.file, JSON.stringify({ accounts: { user: { ...s.entry, refresh_token: 'replacement' } } })); const now = new Date(Date.now() + 2000); fs.utimesSync(s.file, now, now); }
 release(); await expect(pending).rejects.toThrow('changed');
 const data = JSON.parse(fs.readFileSync(s.file, 'utf8')); expect(data.accounts.user?.refresh_token).toBe(action === 'remove' ? undefined : 'replacement');
 } finally { release(); s.cleanup(); }
 });
 it('serves authenticated broker HTTP without leaking errors or caching tokens', async () => {
 const s = setup((async () => Response.json({ access_token: jwt('https://chatsvcagg.teams.microsoft.com'), refresh_token: 'next' })) as typeof fetch);
 const { server } = creaServer({ chiave: 'key', fonti: [], battito: path.join(path.dirname(s.file), 'heartbeat'), tokens: s.manager, zona: 'Europe/Rome', registro: () => {} });
 await new Promise<void>(r => server.listen(0, '127.0.0.1', r));
 const url = 'http://127.0.0.1:' + (server.address() as { port: number }).port + '/api/token-resource?account=user&resource=csa';
 try { expect((await fetch(url)).status).toBe(401); const response = await fetch(url, { headers: { Authorization: 'Bearer key' } }); expect(response.status).toBe(200); expect(response.headers.get('cache-control')).toBe('no-store'); expect(await response.json()).toEqual({ token: jwt('https://chatsvcagg.teams.microsoft.com') });
 const denied = await fetch(url.replace('resource=csa', 'resource=evil'), { headers: { Authorization: 'Bearer key' } }); expect(denied.status).toBe(403); expect(await denied.json()).toEqual({ errore: 'shared token unavailable' });
 } finally { await new Promise<void>(r => server.close(() => r())); s.cleanup(); }
 });
 it('enforces capability isolation and resource allowlist', () => {
 const shared = { tenantId: 'tenant', objectId: 'object', capabilities: ['chat'] as ('chat' | 'mail')[], origin: 'mobile' as const };
 expect(() => allowed(shared, 'outlook')).toThrow('denied'); expect(() => allowed(shared, 'constructor')).toThrow('denied');
 shared.capabilities = ['mail']; expect(() => allowed(shared, 'csa')).toThrow('denied'); expect(allowed(shared, 'outlook')[0]).toBe('https://outlook.office.com');
 });
 it.each(['mobile', 'outlook-spa', 'teams-spa'] as const)('uses explicit %s origin provenance', async origin => {
 let observed: unknown;
 await renewShared({ tenantId: 'tenant', objectId: 'object', capabilities: ['chat'], origin }, 'csa', 'rt', 'client', (async (_u, init) => { observed = (init?.headers as Record<string, string>).Origin; return Response.json({ access_token: jwt('https://chatsvcagg.teams.microsoft.com') }); }) as typeof fetch, 1000);
 expect(observed).toBe(origin === 'mobile' ? undefined : origin === 'teams-spa' ? 'https://teams.microsoft.com' : 'https://outlook.cloud.microsoft');
 });
});

describe("shared regression boundaries", () => {
 it.each(["calendar-only", "refresh-failure"])("does not resurrect %s credentials from legacy file", async kind => {
 const s = setup((async () => Response.json({error: "invalid_grant"}, {status: 400})) as typeof fetch);
 const dir = path.dirname(s.file); const legacy = path.join(dir, "legacy.json");
 const entry = {...s.entry, shared: {...s.entry.shared, capabilities: kind === "calendar-only" ? ["calendar"] : ["mail"]}};
 fs.writeFileSync(s.file, JSON.stringify({accounts: {user: entry}}));
 fs.writeFileSync(legacy, JSON.stringify({account: [{upn: "user", token: "must-not-leak"}]}));
 const {server} = creaServer({chiave: "key", fonti: [], tokenPosta: legacy, battito: path.join(dir, "heartbeat"), tokens: s.manager, zona: "Europe/Rome", registro: () => {}});
 await new Promise<void>(r => server.listen(0, "127.0.0.1", r));
 try { const response = await fetch("http://127.0.0.1:" + (server.address() as {port: number}).port + "/api/token-posta", {headers: {Authorization: "Bearer key"}}); expect(response.status).toBe(200); expect(((await response.json()) as {account: unknown[]}).account).toEqual([]); } finally { await new Promise<void>(r => server.close(() => r())); s.cleanup(); }
 });
 it.each(["object", "other"])("OAuth replacement validates identity %s and drops cache", async oid => {
 const s = setup(fetch);
 const token = "x." + Buffer.from(JSON.stringify({upn: "user", aud: "https://outlook.office.com", tid: "tenant", oid, exp: 9000, scp: "Mail.ReadWrite Calendars.ReadWrite"})).toString("base64url") + ".x";
 try { const manager = s.manager as unknown as {salvaRispostaTokenMicrosoft(json: unknown): unknown};
 if (oid === "other") expect(() => manager.salvaRispostaTokenMicrosoft({access_token: token, refresh_token: "new"})).toThrow("mismatch");
 else { manager.salvaRispostaTokenMicrosoft({access_token: token, refresh_token: "new"}); expect(s.manager.voceAccount("user")?.shared?.capabilities).toEqual(s.entry.shared.capabilities); expect(s.manager.voceAccount("user")?.resourceTokens).toBeUndefined(); }
 expect(s.manager.voceAccount("user")?.refresh_token).toBe(oid === "other" ? "initial" : "new");
 } finally {s.cleanup();}
 });
});

describe("shared SPA recovery", () => {
 it.each([["object", false], ["other", false], ["object", true], ["other", true]] as const)("validates identity %s during import=%s", async (oid, importing) => {
 const token = "x." + Buffer.from(JSON.stringify({upn: "user@example.com", tid: "tenant", oid, aud: "https://outlook.office.com", exp: 9000, scp: "Mail.ReadWrite Calendars.ReadWrite"})).toString("base64url") + ".x";
 const calls: string[] = [];
 const s = setup((async (input, init) => {
 const url = String(input); calls.push(url);
 if (url.includes("/authorize?")) { const state = new URL(url).searchParams.get("state"); return new Response(null, {status: 302, headers: {Location: "https://outlook.cloud.microsoft/mail/?code=code&state=" + state}}); }
 if (new URLSearchParams(String(init?.body)).get("grant_type") === "refresh_token") return Response.json({error: "invalid_grant"}, {status: 400});
 return Response.json({access_token: token, refresh_token: "recovered"});
 }) as typeof fetch);
 const entry = {...s.entry, shared: {...s.entry.shared, origin: "outlook-spa"}, sessione: {userAgent: "test", loginHint: "user@example.com", cookies: [{name: "ESTSAUTHPERSISTENT", value: "cookie", domain: "login.microsoftonline.com", path: "/", expires: -1}]}};
 fs.writeFileSync(s.file, JSON.stringify({accounts: {"user@example.com": entry}}));
 try {
 const pending = importing ? s.manager.importaSessione(entry.sessione, "tenant") : s.manager.ottieniRisorsa("user@example.com", "outlook");
 if (oid === "other") await expect(pending).rejects.toThrow("mismatch");
 else { const result = await pending; expect(result).toEqual(importing ? {account: "user@example.com", tenant_id: "tenant"} : token); expect(s.manager.voceAccount("user@example.com")?.shared?.capabilities).toEqual(entry.shared.capabilities); expect(s.manager.voceAccount("user@example.com")?.resourceTokens?.csa).toBeUndefined(); }
 expect(s.manager.voceAccount("user@example.com")?.refresh_token).toBe(oid === "object" ? "recovered" : "initial");
 expect(calls).toHaveLength(importing ? 2 : 3); expect(calls.every(url => url.startsWith("https://login.microsoftonline.com/"))).toBe(true);
 } finally {s.cleanup();}
 });
});

describe("shared cache permissions and export", () => {
 it("does not reuse cached Outlook tokens missing enabled scopes", async () => {
 let calls = 0; const s = setup((async () => { calls++; return Response.json({access_token: jwt("https://outlook.office.com")}); }) as typeof fetch);
 const invalid = "x." + Buffer.from(JSON.stringify({aud: "https://outlook.office.com", tid: "tenant", oid: "object", exp: 9000, scp: "Mail.Read"})).toString("base64url") + ".x";
 fs.writeFileSync(s.file, JSON.stringify({accounts: {user: {...s.entry, resourceTokens: {outlook: {token: invalid, expiresAt: 9000}}}}}));
 try {expect(await s.manager.ottieniRisorsa("user", "outlook")).not.toBe(invalid); expect(calls).toBe(1);} finally {s.cleanup();}
 });
 it("omits calendar-only token from exported mail file", async () => {
 const s = setup((async () => Response.json({access_token: jwt("https://outlook.office.com")})) as typeof fetch); const output = path.join(path.dirname(s.file), "mail.json");
 fs.writeFileSync(s.file, JSON.stringify({accounts: {user: {...s.entry, shared: {...s.entry.shared, capabilities: ["calendar"]}}}}));
 try {const manager = new GestoreToken(s.file, {fetchFn: s.manager.fetchFn, adesso: () => 1000000, tokenPostaPath: output}); await manager.ottieniRisorsa("user", "outlook"); expect(JSON.parse(fs.readFileSync(output, "utf8")).account).toEqual([]);} finally {s.cleanup();}
 });
});

describe("shared provisioning endpoint", () => {
 it("lists, disables and re-enables sharing over authenticated HTTP, dropping the resource cache", async () => {
 const claimed = (aud: string) => "x." + Buffer.from(JSON.stringify({ aud, oid: "11111111-1111-4111-8111-111111111111", tid: "22222222-2222-4222-8222-222222222222", exp: 9000, upn: "user", scp: "Mail.ReadWrite Calendars.ReadWrite" })).toString("base64url") + ".x";
 const fetchFn = (async (_url, init) => { const body = new URLSearchParams(String(init?.body)); return Response.json({ access_token: claimed(body.get("scope")!.split("/.default")[0]!), refresh_token: "next" }); }) as typeof fetch;
 const s = setup(fetchFn);
 fs.writeFileSync(s.file, JSON.stringify({ accounts: { user: { ...s.entry, shared: { ...s.entry.shared, tenantId: "22222222-2222-4222-8222-222222222222", objectId: "11111111-1111-4111-8111-111111111111" } } } }));
 const { server } = creaServer({ chiave: "key", fonti: [], battito: path.join(path.dirname(s.file), "heartbeat"), tokens: s.manager, zona: "Europe/Rome", registro: () => {} });
 await new Promise<void>(r => server.listen(0, "127.0.0.1", r));
 const base = "http://127.0.0.1:" + (server.address() as { port: number }).port + "/api/auth/shared";
 const auth = { Authorization: "Bearer key", "Content-Type": "application/json" };
 const post = (body: string) => fetch(base, { method: "POST", headers: auth, body });
 try {
 expect((await fetch(base)).status).toBe(401);
 expect((await post("{")).status).toBe(400);
 await s.manager.ottieniRisorsa("user", "csa");
 expect(JSON.parse(fs.readFileSync(s.file, "utf8")).accounts.user.resourceTokens).toBeDefined();
 const off = await (await post(JSON.stringify({ account: "user", enabled: false }))).json() as Record<string, unknown>;
 expect(off.ok).toBe(true);
 const stored = JSON.parse(fs.readFileSync(s.file, "utf8")).accounts.user;
 expect(stored.shared).toBeUndefined(); expect(stored.resourceTokens).toBeUndefined();
 const listed = await (await fetch(base, { headers: auth })).json() as { accounts: { account: string; shared: unknown }[] };
 expect(listed.accounts.find(a => a.account === "user")?.shared).toBeNull();
 const on = await (await post(JSON.stringify({ account: "user", capabilities: ["mail", "chat"], tenantId: "evil", objectId: "evil" }))).json() as Record<string, unknown>;
 expect(on.errore).toBeUndefined(); expect(on.ok).toBe(true);
 const again = JSON.parse(fs.readFileSync(s.file, "utf8")).accounts.user.shared;
 expect(again).toMatchObject({ tenantId: "22222222-2222-4222-8222-222222222222", objectId: "11111111-1111-4111-8111-111111111111", capabilities: ["mail", "chat"] });
 expect((await (await post(JSON.stringify({ account: "user", capabilities: ["bogus"] }))).json() as Record<string, unknown>).ok).toBe(false);
 } finally { await new Promise<void>(r => server.close(() => r())); s.cleanup(); }
 });
});
