import { describe, expect, it } from "vitest";
import { GET, POST } from "../src/app/mcp/route";
import { analizzaDestinatari, formattaCorpoPerHtml, htmlInMarkdown, MCP_TOOLS, sembraHtml } from "../src/lib/mcp/posta";

describe("MCP Posta (/mcp)", () => {
  it("rifiuta richieste senza token o con Origin del browser", async () => {
    process.env.AGENTE_CHIAVE = "segreto-di-prova-123";
    const rOrigin = await POST(
      new Request("http://localhost:3000/mcp", {
        method: "POST",
        headers: { Origin: "https://evil.example", Authorization: "Bearer segreto-di-prova-123" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" }),
      }),
    );
    expect(rOrigin.status).toBe(403);

    const rNoAuth = await POST(
      new Request("http://localhost:3000/mcp", {
        method: "POST",
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" }),
      }),
    );
    expect(rNoAuth.status).toBe(401);

    const rGet = await GET(
      new Request("http://localhost:3000/mcp", {
        headers: { Authorization: "Bearer segreto-di-prova-123" },
      }),
    );
    expect(rGet.status).toBe(405);
  });

  it("risponde a initialize e tools/list con i 10 tool delle email", async () => {
    process.env.AGENTE_CHIAVE = "segreto-di-prova-123";
    const rInit = await POST(
      new Request("http://localhost:3000/mcp", {
        method: "POST",
        headers: { Authorization: "Bearer segreto-di-prova-123", "Content-Type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18" } }),
      }),
    );
    expect(rInit.status).toBe(200);
    const jInit = (await rInit.json()) as { result: { serverInfo: { name: string } } };
    expect(jInit.result.serverInfo.name).toBe("oe-copilot-mail");

    const rList = await POST(
      new Request("http://localhost:3000/mcp", {
        method: "POST",
        headers: { Authorization: "Bearer segreto-di-prova-123", "Content-Type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list" }),
      }),
    );
    expect(rList.status).toBe(200);
    const jList = (await rList.json()) as { result: { tools: { name: string }[] } };
    expect(jList.result.tools.map((t) => t.name)).toEqual(MCP_TOOLS.map((t) => t.name));
    expect(jList.result.tools.length).toBe(10);
  });

  it("converte HTML in Markdown (tabelle, link SafeLinks, grassetto) e preserva a-capo e HTML nelle risposte", () => {
    const md = htmlInMarkdown(
      '<html><body><p>Ciao <b>Marco</b>,</p><table><tr><th>VM</th><th>Stato</th></tr><tr><td>vm-01</td><td>Attiva</td></tr></table>' +
        '<p><a href="https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fexample.org%2Ftest&amp;data=05">Link</a></p></body></html>',
    );
    expect(md).toContain("**Marco**");
    expect(md).toContain("| VM | Stato |");
    expect(md).toContain("| vm-01 | Attiva |");
    expect(md).toContain("[Link](https://example.org/test)");

    expect(sembraHtml("<p>Test</p>")).toBe(true);
    expect(sembraHtml("Riga 1\nRiga 2")).toBe(false);
    expect(formattaCorpoPerHtml("Riga 1\nRiga 2", false)).toContain("Riga 1<br>\nRiga 2");
    expect(formattaCorpoPerHtml("<p><b>Rich</b></p>", true)).toBe("<p><b>Rich</b></p>");

    const recs = analizzaDestinatari(["Mario Rossi <mario@example.com>; luigi@example.com", "mario@example.com"]);
    expect(recs).toEqual([
      { EmailAddress: { Address: "mario@example.com", Name: "Mario Rossi" } },
      { EmailAddress: { Address: "luigi@example.com" } },
    ]);
  });
});
