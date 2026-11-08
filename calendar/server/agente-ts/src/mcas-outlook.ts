import crypto from "node:crypto";
import {
  configuration,
  mcasForm,
  mcasContinuation,
  mcasRestore,
  bootstrapUrl,
  mcasMoved,
} from "./shared/mcas-auth-forms.mjs";
import { leggiTesto, type FetchFn } from "./outlook-rest.ts";
import type { CookieSessione } from "./sessione-web.ts";

export type ProxySession = { bootstrapUrl: string; cookies: CookieSessione[] };
export class ProxyBrowserRequired extends Error {}
const LOGIN = "https://login.microsoftonline.com";
const PROXY = LOGIN + ".mcas.ms";
const APP = "https://outlook.cloud.microsoft";
const REDIRECT = APP + "/mail/";
const CLIENT = "9199bf20-a13f-4107-85dc-02114787ef48";
const SCOPE =
  "https://outlook.office.com/.default offline_access openid profile";

export function proxyCookie(c: CookieSessione): boolean {
  if (
    !c ||
    ![c.name, c.value, c.domain, c.path].every((v) => typeof v === "string") ||
    !Number.isFinite(c.expires) ||
    !/^[!#$%&'*+.^_`|~\w-]+$/.test(c.name) ||
    /[\x00-\x20\x7f;]/.test(c.value) ||
    !c.path.startsWith("/")
  )
    return false;
  const domain = c.domain.replace(/^\./, "").toLowerCase();
  return (
    domain === "login.microsoftonline.com" ||
    domain === "login.microsoftonline.com.mcas.ms" ||
    (domain === "mcas.ms" && /^AdallomAuth\d*$/.test(c.name)) ||
    (/^[a-z0-9-]+\.access\.mcas\.ms$/.test(domain) &&
      c.name === "CAACSkipDisclaimer")
  );
}
function forHost(cookie: CookieSessione, host: string): boolean {
  const domain = cookie.domain.replace(/^\./, "").toLowerCase();
  return (
    host === domain ||
    (cookie.domain.startsWith(".") && host.endsWith("." + domain))
  );
}

export async function authorizeProxy(
  session: ProxySession,
  account: string,
  userAgent: string,
  fetchFn: FetchFn,
): Promise<Record<string, unknown>> {
  try {
    return await proxyFlow(session, account, userAgent, fetchFn);
  } catch (e) {
    if (e instanceof ProxyBrowserRequired) throw e;
    if (
      e instanceof Error &&
      /^(Microsoft security proxy|Unsupported Microsoft)/.test(e.message)
    )
      throw e;
    throw new Error(
      "Microsoft security proxy returned an unsupported response",
    );
  }
}
async function proxyFlow(
  session: ProxySession,
  account: string,
  userAgent: string,
  fetchFn: FetchFn,
): Promise<Record<string, unknown>> {
  const bootstrap = new URL(session.bootstrapUrl);
  if (
    bootstrap.protocol !== "https:" ||
    bootstrap.host !== "mcas-proxyweb.mcas.ms" ||
    bootstrap.pathname !== "/js-bootstrap.js" ||
    bootstrap.username ||
    bootstrap.password
  )
    throw new Error("Unsupported Microsoft security proxy bootstrap");
  session.cookies = session.cookies.filter(proxyCookie);
  let gateway = "";
  const permitted = (value: string) => {
    const url = new URL(value);
    if (
      url.protocol !== "https:" ||
      url.username ||
      url.password ||
      url.port ||
      ![
        "mcas-proxyweb.mcas.ms",
        "login.microsoftonline.com",
        "login.microsoftonline.com.mcas.ms",
        "outlook.cloud.microsoft",
        "outlook.cloud.microsoft.mcas.ms",
        "inline.cdn.mcas.ms",
        gateway,
      ].includes(url.hostname)
    )
      throw new Error("Unsupported Microsoft security proxy destination");
    return url;
  };
  const deadline = Date.now() + 90000;
  const send = async (
    value: string,
    fields?: Record<string, string>,
    headers: Record<string, string> = {},
  ) => {
    const url = permitted(value),
      now = Math.floor(Date.now() / 1000);
    const cookie = session.cookies
      .filter(
        (c) =>
          forHost(c, url.hostname) &&
          (c.expires === -1 || c.expires > now) &&
          (c.path === "/" ||
            url.pathname === c.path ||
            url.pathname.startsWith(
              c.path.endsWith("/") ? c.path : c.path + "/",
            )),
      )
      .sort((a, b) => b.path.length - a.path.length)
      .map((c) => c.name + "=" + c.value)
      .join("; ");
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error("Microsoft security proxy timed out");
    const response = await fetchFn(url, {
      method: fields ? "POST" : "GET",
      redirect: "manual",
      headers: {
        "User-Agent": userAgent,
        ...(cookie ? { Cookie: cookie } : {}),
        ...(fields
          ? { "Content-Type": "application/x-www-form-urlencoded" }
          : {}),
        ...headers,
      },
      body: fields ? new URLSearchParams(fields).toString() : undefined,
      signal: AbortSignal.timeout(Math.min(15000, remaining)),
    });
    for (const raw of response.headers.getSetCookie()) {
      const [pair = "", ...attrs] = raw.split(";");
      const equal = pair.indexOf("=");
      if (equal < 1) continue;
      const c: CookieSessione = {
        name: pair.slice(0, equal).trim(),
        value: pair.slice(equal + 1).trim(),
        domain: url.hostname,
        path: "/",
        expires: -1,
      };
      let maxAge: number | undefined;
      for (const attr of attrs) {
        const [name = "", ...rest] = attr.trim().split("=");
        const v = rest.join("=");
        if (name.toLowerCase() === "domain")
          c.domain = "." + v.replace(/^\./, "");
        if (name.toLowerCase() === "path") c.path = v;
        if (name.toLowerCase() === "expires" && Number.isFinite(Date.parse(v)))
          c.expires = Date.parse(v) / 1000;
        if (name.toLowerCase() === "max-age" && /^-?\d+$/.test(v))
          maxAge = Number(v);
      }
      if (maxAge !== undefined) c.expires = now + maxAge;
      if (
        !proxyCookie(c) ||
        !forHost(
          { ...c, domain: "." + c.domain.replace(/^\./, "") },
          url.hostname,
        )
      )
        continue;
      session.cookies = session.cookies.filter(
        (old) =>
          old.name !== c.name ||
          old.domain.replace(/^\./, "") !== c.domain.replace(/^\./, "") ||
          old.path !== c.path,
      );
      if (c.expires === -1 || c.expires > now) session.cookies.push(c);
    }
    if (response.status === 429 || response.status >= 500)
      throw new Error(`Microsoft security proxy HTTP ${response.status}`);
    return response;
  };
  const text = async (r: Response) => {
    if (!r.ok)
      throw new ProxyBrowserRequired(
        "Microsoft security proxy session requires browser sign-in",
      );
    return leggiTesto(r, 2 * 1024 * 1024);
  };
  const readSettings = async (url: string) => {
    const bootstrap = new URL(url);
    if (
      bootstrap.protocol !== "https:" ||
      bootstrap.host !== "mcas-proxyweb.mcas.ms" ||
      bootstrap.pathname !== "/js-bootstrap.js" ||
      bootstrap.username ||
      bootstrap.password
    )
      throw new Error("Unsupported Microsoft security proxy bootstrap");
    const settings = configuration(
      await text(await send(bootstrap.href)),
      "__JSWRAPPER.initWrapperFixer",
    );
    const unsupported = [
      settings.urlIntegrityEnforcementEnabled !== false && "integrity",
      settings.suffixDomainName !== "mcas.ms" && "suffix",
      settings.niptuckSubDomainName !== "mcas-proxyweb" && "subdomain",
      typeof settings.authUserData !== "string" && "auth context",
      !["string", "number"].includes(typeof settings.topFrameSaasId) &&
        "app id",
    ].filter(Boolean);
    if (unsupported.length)
      throw new ProxyBrowserRequired(
        "Microsoft security proxy context unavailable: " +
          unsupported.join(", "),
      );
    return settings;
  };
  const refreshBootstrap = async () => {
    let root = await send(APP + ".mcas.ms/mail/");
    for (let i = 0; i < 2 && root.status >= 300 && root.status < 400; i++) {
      const next = permitted(
        new URL(root.headers.get("Location") || "", APP + ".mcas.ms/mail/")
          .href,
      );
      if (next.origin !== APP + ".mcas.ms")
        throw new Error("Microsoft security proxy app context changed");
      root = await send(next.href);
    }
    const next = bootstrapUrl(await text(root), APP + ".mcas.ms");
    if (next) session.bootstrapUrl = next;
  };
  const settings = await readSettings(bootstrap.href);
  const proxied = (value: string, xhr = false) => {
    const u = new URL(value);
    if (
      settings.extendedTopFrameSaasIdParamEnabled ||
      (!xhr && settings.topFrameSaasIdParamEnabled)
    )
      u.searchParams.set("McasTsid", String(settings.topFrameSaasId));
    if (xhr || settings.resolveNavigationRequestByUrlEnabled)
      u.searchParams.set("McasCtx", xhr ? "2" : "4");
    if (xhr) u.searchParams.set("McasUserAuth", String(settings.authUserData));
    return u.href;
  };
  const verifier = crypto.randomBytes(32).toString("base64url"),
    state = crypto.randomBytes(24).toString("base64url");
  const auth = new URL(PROXY + "/common/oauth2/v2.0/authorize");
  auth.search = new URLSearchParams({
    client_id: CLIENT,
    redirect_uri: REDIRECT,
    response_type: "code",
    response_mode: "query",
    scope: SCOPE,
    login_hint: account,
    state,
    code_challenge: crypto
      .createHash("sha256")
      .update(verifier)
      .digest("base64url"),
    code_challenge_method: "S256",
  }).toString();
  let current = auth.href,
    response = await send(proxied(current)),
    code = "",
    resumed = false;
  for (let i = 0; i < 14; i++) {
    const from = permitted(current),
      location = response.headers.get("Location");
    if (location) {
      const next = permitted(new URL(location, from).href);
      const p = next.searchParams.has("state")
        ? next.searchParams
        : new URLSearchParams(next.hash.slice(1));
      if (p.has("code") || p.has("error")) {
        if (
          next.origin.replace(/\.mcas\.ms$/, "") !== APP ||
          next.pathname !== "/mail/" ||
          p.get("state") !== state
        )
          throw new Error("Microsoft security proxy callback mismatch");
        if (p.has("error"))
          throw new Error("Microsoft security proxy session requires sign-in");
        code = p.get("code") || "";
        break;
      }
      if (next.hostname === "inline.cdn.mcas.ms") {
        const step = mcasRestore(next.href, gateway);
        current = step.action;
        response = await send(current, step.fields, { Origin: next.origin });
      } else {
        current = next.href;
        response = await send(current);
      }
      continue;
    }
    const html = await text(response);
    if (
      from.hostname === gateway &&
      from.pathname === "/finalize_login" &&
      /<title>\s*Object Moved\s*<\/title>/i.test(html)
    ) {
      current = mcasMoved(html, APP);
      response = await send(current);
      continue;
    }
    if (html.includes("PROXYWEB_MONITORED_ACCESS_MESSAGE")) {
      const step = mcasContinuation(html);
      if (new URL(step.action).hostname !== gateway)
        throw new Error("Microsoft security proxy gateway changed");
      current = step.action;
      response = await send(current, step.fields, {
        Origin: from.origin,
        Referer: from.origin + "/",
      });
      continue;
    }
    if (/<form\b/i.test(html)) {
      if (from.origin !== LOGIN)
        throw new Error("Unsupported Microsoft security proxy form issuer");
      const step = mcasForm(html);
      const host = new URL(step.action).hostname;
      if (gateway && gateway !== host)
        throw new Error("Microsoft security proxy gateway changed");
      gateway = host;
      current = step.action;
      response = await send(
        current,
        { id_token: step.idToken },
        { Origin: from.origin, Referer: from.origin + "/" },
      );
      continue;
    }
    const c = configuration(html, "$Config");
    const sessions = Array.isArray(c.arrSessions)
      ? c.arrSessions.filter(
          (v: {
            isSignedIn?: boolean;
            matchesLoginHint?: boolean;
            name?: string;
            id?: string;
          }) =>
            v?.isSignedIn &&
            v.matchesLoginHint &&
            v.name?.toLowerCase() === account &&
            v.id,
        )
      : [];
    if (
      !resumed &&
      String(c.sErrorCode) === "50058" &&
      c.fSsoFeasible === true &&
      sessions.length === 1 &&
      typeof c.urlRefresh === "string"
    ) {
      const next = permitted(new URL(c.urlRefresh, from).href);
      if (
        next.origin !== LOGIN ||
        !/^\/(common|[a-f0-9-]{36})\/reprocess$/.test(next.pathname)
      )
        throw new Error("Unsupported Microsoft session selection");
      next.searchParams.set("sessionid", sessions[0].id);
      resumed = true;
      current = next.href;
      response = await send(current);
      continue;
    }
    throw new Error("Microsoft security proxy requires interactive sign-in");
  }
  if (!code)
    throw new Error("Microsoft security proxy continuation limit reached");
  let target = proxied(PROXY + "/common/oauth2/v2.0/token", true),
    origin = APP + ".mcas.ms";
  const form = {
    client_id: CLIENT,
    redirect_uri: REDIRECT,
    grant_type: "authorization_code",
    code,
    code_verifier: verifier,
    scope: SCOPE,
  };
  for (let i = 0; i < 3; i++) {
    response = await send(target, form, { Origin: origin });
    if (response.status !== 307) break;
    const next = permitted(
      new URL(response.headers.get("Location") || "", target).href,
    );
    if (next.origin !== LOGIN || next.pathname !== "/common/oauth2/v2.0/token")
      throw new Error("Unsupported Microsoft token redirect");
    target = next.href;
    origin = "null";
  }
  const result = JSON.parse(await text(response)) as Record<string, unknown>;
  if (
    typeof result.access_token !== "string" ||
    typeof result.refresh_token !== "string"
  )
    throw new Error("Microsoft security proxy token missing");
  await refreshBootstrap();
  return result;
}
