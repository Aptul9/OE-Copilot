import crypto from "node:crypto";
import { config } from "@/lib/config";
import { eseguiToolMcp, MCP_TOOLS } from "@/lib/mcp/posta";

export const dynamic = "force-dynamic";

const PROTOCOL_VERSION = "2025-06-18";

function tokenValido(authHeader: string | null): boolean {
  const segreto = config.mcpToken;
  if (!segreto || !authHeader) return false;
  const m = /^Bearer\s+(\S+)\s*$/i.exec(authHeader);
  if (!m?.[1]) return false;
  const a = Buffer.from(m[1], "utf8");
  const b = Buffer.from(segreto, "utf8");
  return a.length === b.length && crypto.timingSafeEqual(a, b);
}

function nonAutorizzato(): Response {
  return Response.json(
    { jsonrpc: "2.0", error: { code: -32001, message: "Missing or invalid Bearer token" }, id: null },
    { status: 401, headers: { "WWW-Authenticate": 'Bearer realm="oe-copilot"', "Cache-Control": "no-store" } },
  );
}

interface RichiestaRpc {
  jsonrpc?: string;
  id?: unknown;
  method?: string;
  params?: Record<string, unknown>;
}

async function gestisciMessaggio(req: RichiestaRpc): Promise<Record<string, unknown> | null> {
  if (req.id === undefined || req.id === null) {
    // Notifica JSON-RPC (es. notifications/initialized)
    return null;
  }
  const id = req.id;
  const method = req.method ?? "";
  const params = req.params && typeof req.params === "object" ? req.params : {};

  if (method === "initialize") {
    const version = typeof params.protocolVersion === "string" && params.protocolVersion ? params.protocolVersion : PROTOCOL_VERSION;
    return {
      jsonrpc: "2.0",
      id,
      result: {
        protocolVersion: version,
        capabilities: { tools: {} },
        serverInfo: { name: "oe-copilot-mail", version: "1.0.0" },
        instructions:
          "OE Copilot Mail MCP: read and send emails across all Outlook accounts signed in on the OE Copilot virtual phone. " +
          "Supports reading full conversation threads, searching, downloading/expanding attachments, and sending/replying/forwarding " +
          "with rich HTML formatting, selective recipients (to, cc, replace_cc, exclude_recipients), and attachments.",
      },
    };
  }

  if (method === "ping") {
    return { jsonrpc: "2.0", id, result: {} };
  }

  if (method === "tools/list") {
    return { jsonrpc: "2.0", id, result: { tools: MCP_TOOLS } };
  }

  if (method === "tools/call") {
    const nome = typeof params.name === "string" ? params.name : "";
    const tool = MCP_TOOLS.find((t) => t.name === nome);
    if (!tool) {
      return { jsonrpc: "2.0", id, error: { code: -32602, message: `Unknown tool: ${nome}` } };
    }
    const args = params.arguments && typeof params.arguments === "object" && !Array.isArray(params.arguments) ? (params.arguments as Record<string, unknown>) : {};
    try {
      const content = await eseguiToolMcp(nome, args);
      return { jsonrpc: "2.0", id, result: { content } };
    } catch (e) {
      const msg = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
      return {
        jsonrpc: "2.0",
        id,
        result: { content: [{ type: "text", text: `Error: ${msg}` }], isError: true },
      };
    }
  }

  return { jsonrpc: "2.0", id, error: { code: -32601, message: `Method not found: ${method}` } };
}

export async function POST(req: Request): Promise<Response> {
  if (req.headers.has("origin")) {
    return Response.json({ error: "Richieste da pagine web non ammesse" }, { status: 403, headers: { "Cache-Control": "no-store" } });
  }
  if (!tokenValido(req.headers.get("authorization"))) return nonAutorizzato();

  let payload: unknown;
  try {
    payload = await req.json();
  } catch {
    return Response.json(
      { jsonrpc: "2.0", error: { code: -32700, message: "Parse error" }, id: null },
      { status: 400, headers: { "Cache-Control": "no-store" } },
    );
  }

  if (Array.isArray(payload)) {
    const risposte: Record<string, unknown>[] = [];
    for (const item of payload) {
      if (item && typeof item === "object") {
        const r = await gestisciMessaggio(item as RichiestaRpc);
        if (r) risposte.push(r);
      }
    }
    if (risposte.length === 0) return new Response(null, { status: 202, headers: { "Cache-Control": "no-store" } });
    return Response.json(risposte, { status: 200, headers: { "Cache-Control": "no-store" } });
  }

  if (!payload || typeof payload !== "object") {
    return Response.json(
      { jsonrpc: "2.0", error: { code: -32600, message: "Invalid Request" }, id: null },
      { status: 400, headers: { "Cache-Control": "no-store" } },
    );
  }

  const r = await gestisciMessaggio(payload as RichiestaRpc);
  if (!r) return new Response(null, { status: 202, headers: { "Cache-Control": "no-store" } });
  return Response.json(r, { status: 200, headers: { "Cache-Control": "no-store" } });
}

export async function GET(req: Request): Promise<Response> {
  if (!tokenValido(req.headers.get("authorization"))) return nonAutorizzato();
  return new Response(null, { status: 405, headers: { Allow: "POST, DELETE", "Cache-Control": "no-store" } });
}

export async function DELETE(req: Request): Promise<Response> {
  if (!tokenValido(req.headers.get("authorization"))) return nonAutorizzato();
  return new Response(null, { status: 204, headers: { "Cache-Control": "no-store" } });
}
