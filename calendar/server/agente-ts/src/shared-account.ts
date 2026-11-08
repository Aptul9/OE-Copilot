import { claimsJwt, type FetchFn } from './outlook-rest.ts';
export type SharedAccount = { tenantId: string; objectId: string; capabilities: ('mail' | 'calendar' | 'chat')[]; origin: 'mobile' | 'outlook-spa' | 'teams-spa' };
const resources: Record<string, string[]> = {
 outlook: ['https://outlook.office.com', 'https://outlook.office365.com', '00000002-0000-0ff1-ce00-000000000000'],
 csa: ['https://chatsvcagg.teams.microsoft.com'], ic3: ['https://ic3.teams.office.com'], spaces: ['https://api.spaces.skype.com'], presence: ['https://presence.teams.microsoft.com'],
};
export function allowed(s: SharedAccount, r: string): string[] {
 if (!s.tenantId || !s.objectId || !Array.isArray(s.capabilities) || !['mobile', 'outlook-spa', 'teams-spa'].includes(s.origin)) throw new Error('invalid shared identity');
 const aud = Object.hasOwn(resources, r) ? resources[r] : undefined;
 if (!aud || !(r === 'outlook' ? s.capabilities.some(c => c === 'mail' || c === 'calendar') : s.capabilities.includes('chat'))) throw new Error('resource or capability denied');
 return aud;
}
export function hasSharedPermissions(s: SharedAccount, r: string, token: string): boolean {
 if (r !== "outlook") return true;
 const scopes = String(claimsJwt(token).scp || "").split(/\s+/);
 return s.capabilities.every(cap => cap === "chat" || scopes.some(scope => cap === "mail" ? /^Mail\.(Read|ReadWrite)(\.Shared)?$/i.test(scope) : /^Calendars\.(Read|ReadWrite)(\.Shared)?$/i.test(scope)));
}
// Teams desktop and mobile: a member of the Microsoft family of clients (FOCI), like Outlook Mobile. Teams calling (authsvc
// authz) answers 410 ApiRestricted to a token of Outlook Mobile and 200 to one of this app, and the chat list of Teams is
// complete only for it (live, 2026-11-08).
export const TEAMS_APP_CLIENT_ID = '1fec8e78-bce4-4aaf-ab1b-5451cc387264';
/** The client a resource of a shared account is asked as: the Teams resources of an Outlook Mobile sign-in (a family refresh token) as Teams itself; anything else as the client of the sign-in. */
export function clientFor(s: SharedAccount, r: string, stored: string): string {
 return s.origin === 'mobile' && r !== 'outlook' ? TEAMS_APP_CLIENT_ID : stored;
}
export class SharedRefreshExpired extends Error {}
export class SharedAuthorizationRefused extends Error {}
export async function renewShared(s: SharedAccount, r: string, refresh: string, client: string, fetchFn: FetchFn, now: number) {
 const aud = allowed(s, r);
 const headers: Record<string, string> = { 'Content-Type': 'application/x-www-form-urlencoded' };
 if (s.origin !== 'mobile') headers.Origin = s.origin === 'teams-spa' ? 'https://teams.microsoft.com' : 'https://outlook.cloud.microsoft';
 const response = await fetchFn('https://login.microsoftonline.com/' + encodeURIComponent(s.tenantId) + '/oauth2/v2.0/token', {
 method: 'POST', headers, redirect: 'error', signal: AbortSignal.timeout(15000),
 body: new URLSearchParams({ client_id: client, grant_type: 'refresh_token', refresh_token: refresh, scope: aud[0] + '/.default offline_access openid profile' }).toString(),
 });
 const j = await response.json() as Record<string, unknown>;
 if (!response.ok && (j.error === 'invalid_grant' || j.error === 'interaction_required')) throw new SharedRefreshExpired('shared refresh expired');
 if (!response.ok && response.status >= 400 && response.status < 500 && response.status !== 429 && j.error !== 'temporarily_unavailable' && j.error !== 'server_error') throw new SharedAuthorizationRefused('Microsoft refused this client for the requested service');
 if (!response.ok || typeof j.access_token !== 'string') throw new Error('shared renewal refused');
 const c = claimsJwt(j.access_token);
 const exp = Number(c.exp);
 if (!hasSharedPermissions(s, r, j.access_token) || c.tid !== s.tenantId || c.oid !== s.objectId || !aud.includes(String(c.aud).replace(/\/$/, '')) || !Number.isFinite(exp) || exp <= now + 120) throw new Error('shared identity, audience or expiry mismatch');
 return { token: j.access_token, expiresAt: exp, refresh: typeof j.refresh_token === 'string' && j.refresh_token ? j.refresh_token : refresh };
}
