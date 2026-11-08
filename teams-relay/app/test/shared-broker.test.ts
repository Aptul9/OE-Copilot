import { afterEach, expect, it, vi } from 'vitest';
import { LightToken, CHATSVCAGG, IC3 } from '../src/agent/light/token';
import { LightApi, notificationsUrl } from '../src/agent/light/api';
import { loadConfig } from '../src/agent/config';
afterEach(() => vi.unstubAllGlobals());
it('explicit broker mode maps scopes, never reads local credential', async () => {
 const requests: URL[] = [];
 vi.stubGlobal('fetch', vi.fn(async (url: URL, init: RequestInit) => { requests.push(url); expect(init.redirect).toBe('error'); expect(init.headers).toEqual({ Authorization: 'Bearer key' }); return Response.json({ token: 'resource-token' }); }));
 const token = new LightToken('missing-local-file', Date.now, { url: 'http://agent:8765', key: 'key', account: 'user', region: 'emea' });
 expect(token.stored()).toBeNull(); expect(token.brokerBacked).toBe(true);
 expect(await token.accessToken(CHATSVCAGG)).toBe('resource-token'); await token.accessToken(IC3);
 expect(requests.map(u => u.searchParams.get('resource'))).toEqual(['csa', 'ic3']);
 expect(requests[0].pathname).toBe('/api/token-resource'); expect(requests[0].searchParams.get('account')).toBe('user');
});
it('broker failure rejects without local fallback', async () => {
 const call = vi.fn(async () => Response.json({}, { status: 403 })); vi.stubGlobal('fetch', call);
 const token = new LightToken('missing', Date.now, { url: 'http://agent', key: 'key', account: 'user', region: 'emea' });
 await expect(token.accessToken(CHATSVCAGG)).rejects.toThrow('shared broker unavailable'); expect(call).toHaveBeenCalledTimes(1);
 await expect(token.accessToken('https://evil.invalid/.default')).rejects.toThrow('unsupported'); expect(call).toHaveBeenCalledTimes(1);
});
it('partial broker configuration fails closed', () => {
 expect(() => loadConfig({ SHARED_BROKER_URL: 'http://agent' })).toThrow('requires');
 // a region is part of the mapping: without it the chats would be read in the wrong geography
 expect(() => loadConfig({ SHARED_BROKER_URL: 'http://agent', SHARED_BROKER_KEY: 'k', SHARED_BROKER_ACCOUNT: 'a@b.example' })).toThrow('region');
 expect(() => loadConfig({ SHARED_BROKER_URL: 'http://agent', SHARED_BROKER_KEY: 'k', SHARED_BROKER_ACCOUNT: 'a@b.example', SHARED_BROKER_REGION: 'AMER/../x' })).toThrow();
});
it('a shared account outside EMEA reads its own region, never the emea default nor a local file', async () => {
 const c = loadConfig({ SHARED_BROKER_URL: 'http://agent', SHARED_BROKER_KEY: 'k', SHARED_BROKER_ACCOUNT: 'a@b.example', SHARED_BROKER_REGION: 'amer' });
 expect(c.sharedBroker).toEqual({ url: 'http://agent', key: 'k', account: 'a@b.example', region: 'amer' });
 expect(c.alerts.signIn).toContain('a@b.example');
 const token = new LightToken('missing', Date.now, c.sharedBroker);
 expect(token.region()).toBe('amer');
 const urls: string[] = [];
 vi.stubGlobal('fetch', vi.fn(async (url: URL | string) => {
  urls.push(String(url));
  return String(url).includes('/api/token-resource') ? Response.json({ token: 'resource-token' }) : Response.json({ messages: [] });
 }));
 await new LightApi(token, () => token.region()).notifications();
 expect(urls.at(-1)).toBe(notificationsUrl('amer'));
 expect(urls.some((u) => u.includes('/emea/'))).toBe(false);
});
it('without a broker the region stays the one the browser captured', () => {
 const token = new LightToken('missing');
 expect(token.region()).toBe('emea');
});
it('reuses a broker token until shortly before it ends, and a cached one while the agent restarts', async () => {
 const exp = Math.floor(Date.now() / 1000) + 3600;
 const jwt = 'x.' + Buffer.from(JSON.stringify({ exp })).toString('base64url') + '.x';
 const call = vi.fn(async () => Response.json({ token: jwt }));
 vi.stubGlobal('fetch', call);
 let now = Date.now();
 const token = new LightToken('missing', () => now, { url: 'http://agent', key: 'key', account: 'user', region: 'emea' });
 expect(await token.accessToken(CHATSVCAGG)).toBe(jwt);
 expect(await token.accessToken(CHATSVCAGG)).toBe(jwt);
 expect(call).toHaveBeenCalledTimes(1);
 // inside the last five minutes: asked again; the agent not reachable then, the cached one still serves
 now = exp * 1000 - 60_000;
 vi.stubGlobal('fetch', vi.fn(async () => { throw new TypeError('fetch failed'); }));
 expect(await token.accessToken(CHATSVCAGG)).toBe(jwt);
 now = exp * 1000 + 1;
 await expect(token.accessToken(CHATSVCAGG)).rejects.toThrow('not reachable');
});
