import { config } from "@/lib/config";
import { savedSince } from "@/lib/everflow-saved";
import { HttpError, route } from "@/lib/http";
import { mcpUserId, tokenMatches } from "@/lib/mcp/access";

export const dynamic = "force-dynamic";

const integer = (v: string | null, min: number, max: number): number | null => {
  if (v === null || !/^\d+$/.test(v)) return null;
  const n = Number(v);
  return Number.isSafeInteger(n) && n >= min && n <= max ? n : null;
};

// Saved Teams messages of the administrator of .env since a time, for Everflow through the communication service
// (OE-Copilot mcp, /everflow/teams). Bearer MCP_TOKEN only, like /mcp: no session cookie, no web page (Origin).
// ?since=<ms>&max_rows=<n>: {"slots": [{"slot", "snapshot"} | {"slot", "error"}]}, active slots in slot order.
export const GET = route(async (req) => {
  if (!config.mcpToken) throw new HttpError(404, "Not found");
  if (req.headers.has("origin")) throw new HttpError(403, "Requests from web pages are not accepted");
  if (!tokenMatches(req.headers.get("authorization"))) throw new HttpError(401, "Missing or wrong token", { "WWW-Authenticate": 'Bearer realm="teamsrelay"' });
  const query = new URL(req.url).searchParams;
  const since = integer(query.get("since"), 0, Number.MAX_SAFE_INTEGER);
  const maxRows = integer(query.get("max_rows"), 1, 100000);
  if (since === null || maxRows === null) throw new HttpError(400, "since (ms) and max_rows (1 to 100000) are required");
  const userId = mcpUserId();
  if (!userId) throw new HttpError(503, "The administrator of .env does not exist");
  return Response.json(savedSince(userId, since, maxRows));
});
