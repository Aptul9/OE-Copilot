import fs from "node:fs";
import path from "node:path";
import { expect, it, vi } from "vitest";
import { directory } from "@/supervisor/broker";
import { tempDir } from "./helpers";

it("imports a matching saved session through the narrow agent route, never returning cookies to the web app", async () => {
  const root = tempDir();
  fs.mkdirSync(path.join(root, "3"));
  fs.writeFileSync(
    path.join(root, "3/session.json"),
    JSON.stringify({
      loginHint: "user@example.com",
      userAgent: "Chrome",
      cookies: [
        {
          name: "ESTSAUTH",
          value: "private-cookie",
          domain: "login.microsoftonline.com",
          path: "/",
          expires: -1,
        },
      ],
    }),
  );
  const fetchFn = vi.fn(async (_url, init) => {
    expect(JSON.parse(init.body)).toMatchObject({
      operation: "signin-import",
      account: "user@example.com",
      sessione: { loginHint: "user@example.com" },
    });
    return Response.json({ ok: true, route: "outlook-web" });
  });
  const broker = directory({ url: "http://agent", key: "key" }, fetchFn, root);
  expect(
    await broker.connect!({ id: "job", account: "user@example.com", slot: 3 }),
  ).toEqual({ ok: true, route: "outlook-web" });
  // a session saved by an earlier sign-in of another address: the browser sign-in, not a dead end
  expect(
    await broker.connect!({ id: "job", account: "other@example.com", slot: 3 }),
  ).toEqual({ ok: false, interaction: true });
  expect(fetchFn).toHaveBeenCalledTimes(1);
});

it("opens the required Outlook browser context once when silent SSO needs proxy confirmation", async () => {
  const root = tempDir();
  fs.mkdirSync(path.join(root, "3"));
  fs.writeFileSync(
    path.join(root, "3/session.json"),
    JSON.stringify({
      loginHint: "user@example.com",
      userAgent: "Chrome",
      cookies: [],
    }),
  );
  const capture = vi.fn(async () => ({
    bootstrapUrl: "https://mcas-proxyweb.mcas.ms/js-bootstrap.js",
    cookies: [],
  }));
  let attempts = 0;
  const fetchFn = vi.fn(async (_url, init) => {
    if (++attempts === 1)
      return Response.json(
        { ok: false, needsProxy: true, errore: "Proxy session required" },
        { status: 400 },
      );
    expect(JSON.parse(init.body).sessione.proxy.bootstrapUrl).toContain(
      "mcas-proxyweb.mcas.ms",
    );
    return Response.json({ ok: true, route: "outlook-web" });
  });
  const broker = directory(
    { url: "http://agent", key: "key" },
    fetchFn,
    root,
    undefined,
    capture,
  );
  expect(
    await broker.connect!({ id: "job", account: "user@example.com", slot: 3 }),
  ).toMatchObject({ ok: true });
  expect(capture).toHaveBeenCalledTimes(1);
  expect(fetchFn).toHaveBeenCalledTimes(2);
});
