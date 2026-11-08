import { expect, it } from "vitest";
import { authorizeProxy } from "../src/mcas-outlook.ts";
import { mcasMoved } from "../src/shared/mcas-auth-forms.mjs";
it("accepts Outlook's pinned context-restore redirect, rejects foreign application destinations", () => {
  const html =
    '<title>Object Moved</title><script>window.location.replace("https://outlook.cloud.microsoft.mcas.ms/mail/")</script>';
  expect(mcasMoved(html, "https://outlook.cloud.microsoft")).toBe(
    "https://outlook.cloud.microsoft.mcas.ms/mail/",
  );
  expect(() =>
    mcasMoved(
      html.replace("outlook.cloud.microsoft.mcas.ms", "evil.example"),
      "https://outlook.cloud.microsoft",
    ),
  ).toThrow();
});
it.each([
  "http://mcas-proxyweb.mcas.ms/js-bootstrap.js",
  "https://evil.example/js-bootstrap.js",
  "https://mcas-proxyweb.mcas.ms@evil.example/js-bootstrap.js",
])(
  "rejects untrusted bootstrap destinations before sending cookies: %s",
  async (bootstrapUrl) => {
    let calls = 0;
    await expect(
      authorizeProxy(
        { bootstrapUrl, cookies: [] },
        "a@b",
        "Chrome",
        async () => {
          calls++;
          return new Response();
        },
      ),
    ).rejects.toThrow();
    expect(calls).toBe(0);
  },
);

it("proxy session renews its Outlook context and rotates domain cookies", async () => {
  const calls: string[] = [];
  let state = "";
  const session = {
    bootstrapUrl: "https://mcas-proxyweb.mcas.ms/js-bootstrap.js?fixture=1",
    cookies: [
      {
        name: "ESTSAUTH",
        value: "private",
        domain: "login.microsoftonline.com",
        path: "/",
        expires: -1,
      },
    ],
  };
  const fetchFn: typeof fetch = async (input, init) => {
    const url = new URL(String(input));
    calls.push(url.hostname);
    if (url.pathname === "/js-bootstrap.js")
      return new Response(
        '__JSWRAPPER.initWrapperFixer({"urlIntegrityEnforcementEnabled":false,"suffixDomainName":"mcas.ms","niptuckSubDomainName":"mcas-proxyweb","authUserData":"context","topFrameSaasId":1});',
      );
    if (url.pathname.endsWith("/authorize")) {
      state = url.searchParams.get("state")!;
      if (url.hostname.endsWith(".mcas.ms"))
        return new Response(null, {
          status: 302,
          headers: {
            Location:
              "https://login.microsoftonline.com" + url.pathname + url.search,
          },
        });
      return new Response(
        '<form method="post" action="https://tenant.access.mcas.ms/aad_login"><input type="hidden" name="id_token" value="private-id"></form>',
      );
    }
    if (url.pathname === "/aad_login")
      return new Response(null, {
        status: 302,
        headers: {
          Location: `https://outlook.cloud.microsoft.mcas.ms/mail/?code=fixture&state=${state}`,
          "Set-Cookie": "AdallomAuth=rotated; Domain=mcas.ms; Path=/; Secure",
        },
      });
    if (url.pathname === "/mail/") {
      expect(new Headers(init?.headers).get("cookie")).toContain(
        "AdallomAuth=rotated",
      );
      return new Response(
        '<script src="https://mcas-proxyweb.mcas.ms/js-bootstrap.js?fixture=2"></script>',
      );
    }
    expect(String(init?.body)).toContain("code=fixture");
    return Response.json({ access_token: "access", refresh_token: "refresh" });
  };
  expect(
    await authorizeProxy(session, "user@example.com", "Chrome", fetchFn),
  ).toMatchObject({ access_token: "access" });
  expect(calls).toContain("tenant.access.mcas.ms");
  expect(session.bootstrapUrl).toContain("fixture=2");
});
