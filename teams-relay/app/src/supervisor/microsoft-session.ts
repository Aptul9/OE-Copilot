import { chromium, type Browser, type Page } from "playwright-core";
import type { MicrosoftSession } from "../agent/light/session-auth";
import { bootstrapUrl } from "../../scripts/mcas-auth-forms.mjs";

// Captures Microsoft SSO even when Teams is not licensed or has not finished loading.
export async function microsoftSession(
  port: number,
  account: string,
): Promise<MicrosoftSession | null> {
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
    timeout: 5000,
  });
  try {
    const context = browser.contexts()[0];
    const page = context?.pages()[0];
    if (!context || !page) return null;
    const cookies = (
      await context.cookies("https://login.microsoftonline.com")
    ).filter((c) => /^\.?login\.microsoftonline\.com$/i.test(c.domain));
    if (!cookies.some((c) => /^ESTSAUTH(?:PERSISTENT|LIGHT)?$/.test(c.name)))
      return null;
    return {
      loginHint: account,
      cookies,
      userAgent: await page.evaluate(() => navigator.userAgent),
    };
  } finally {
    await browser.close();
  }
}

const outlookSignIns = new Map<
  number,
  { browser: Browser; page: Page; timer: NodeJS.Timeout }
>();
async function releaseOutlook(port: number) {
  const owned = outlookSignIns.get(port);
  if (!owned) return;
  outlookSignIns.delete(port);
  clearTimeout(owned.timer);
  await owned.page.close().catch(() => undefined);
  await owned.browser.close().catch(() => undefined);
}
export async function outlookProxySession(port: number) {
  let owned = outlookSignIns.get(port);
  if (owned && (!owned.browser.isConnected() || owned.page.isClosed())) {
    await releaseOutlook(port);
    owned = undefined;
  }
  const browser =
    owned?.browser ??
    (await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
      timeout: 5000,
    }));
  try {
    const context = browser.contexts()[0];
    if (!context) return null;
    const existing = context
      .pages()
      .find((p) =>
        /^https:\/\/outlook\.cloud\.microsoft(?:\.mcas\.ms)?\//.test(p.url()),
      );
    const page = owned?.page ?? existing ?? (await context.newPage());
    if (!owned && !existing) {
      const timer = setTimeout(() => void releaseOutlook(port), 1800_000);
      timer.unref();
      outlookSignIns.set(port, { browser, page, timer });
      await page
        .goto("https://outlook.cloud.microsoft/mail/", {
          waitUntil: "domcontentloaded",
          timeout: 30000,
        })
        .catch(() => undefined);
    }
    const here = new URL(page.url());
    if (here.origin !== "https://outlook.cloud.microsoft.mcas.ms") return null;
    const result = await context.request.get(here.origin + "/mail/", {
      maxRedirects: 0,
      timeout: 15000,
    });
    if (!result.ok()) return null;
    const html = await result.text();
    if (Buffer.byteLength(html) > 2 * 1024 * 1024) return null;
    const url = bootstrapUrl(html, here.origin);
    if (!url) return null;
    const captured = { bootstrapUrl: url, cookies: await context.cookies() };
    if (outlookSignIns.has(port)) await releaseOutlook(port);
    return captured;
  } finally {
    if (!outlookSignIns.has(port)) await browser.close();
  }
}
