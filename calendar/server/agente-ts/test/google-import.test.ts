import fs from "node:fs";
import { expect, it } from "vitest";
import { captureGoogleSession, importGoogle } from "../scripts/import-google.ts";

it("imports file credentials without putting secrets in URL, logs, or redirects", async () => {
  const seen: { url: string; headers: Headers; body: unknown; redirect: string | undefined }[] = [];
  const result = await importGoogle({ url: "http://127.0.0.1:3180", key: "private-api-key", account: "me@example.com", provider: "gmail", data: { client_id: "client", refresh_token: "private-refresh" }, fetchFn: async (input, init) => {
    seen.push({ url: String(input), headers: new Headers(init?.headers), body: JSON.parse(String(init?.body)), redirect: init?.redirect });
    return new Response('{"ok":true,"account":"me@example.com","provider":"gmail"}');
  } });
  expect(result).toEqual({ account: "me@example.com", provider: "gmail" });
  expect(seen[0]?.url).toBe("http://127.0.0.1:3180/api/auth/completa");
  expect(seen[0]?.headers.get("Authorization")).toBe("Bearer private-api-key");
  expect(seen[0]?.redirect).toBe("error");
  expect(seen[0]?.body).toEqual({ provider: "gmail", account: "me@example.com", credentials: { client_id: "client", refresh_token: "private-refresh" } });
});

it("rejects non-loopback HTTP, ignores error response content", async () => {
  let calls = 0;
  const options = { url: "http://example.com", key: "k", account: "me@example.com", provider: "gchat" as const, data: {}, fetchFn: (async () => { calls++; return new Response("private upstream text", { status: 403 }); }) as typeof fetch };
  await expect(importGoogle(options)).rejects.toThrow(/HTTPS/);
  expect(calls).toBe(0);
  await expect(importGoogle({ ...options, url: "https://example.com" })).rejects.toThrow("Google import HTTP 403");
});

it("captures through existing Playwriter profile, closes owned tab/session, removes temporary cookies", () => {
  const calls: string[][] = [];
  let output = "";
  const data = captureGoogleSession("install:Chrome:profile", args => {
    calls.push(args);
    if (args[0] === "session" && args[1] === "new") return "Session 77 created";
    if (args.includes("-f")) {
      const script = fs.readFileSync(args[args.indexOf("-f") + 1]!, "utf8");
      expect(script).toContain("state.page.close()");
      output = JSON.parse(/const out = (.+);/.exec(script)![1]!);
      fs.writeFileSync(output, '{"user_agent":"test","cookies":[]}');
    }
    return "";
  });
  expect(data).toEqual({ user_agent: "test", cookies: [] });
  expect(calls.at(-1)).toEqual(["session", "delete", "77"]);
  expect(fs.existsSync(output)).toBe(false);
  expect(() => captureGoogleSession("invalid & command", () => "")).toThrow(/browser/);
});
