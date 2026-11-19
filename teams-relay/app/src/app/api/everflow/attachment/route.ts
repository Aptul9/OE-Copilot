import { MEDIA_EXT } from "@/agent/logic/files";
import { appDb, slotsOf } from "@/lib/appdb";
import { config } from "@/lib/config";
import { inlineDisposition } from "@/lib/files";
import { HttpError, route } from "@/lib/http";
import { mcpUserId, tokenMatches } from "@/lib/mcp/access";
import { attachmentsOf, fetchAttachment } from "@/lib/mcp/tools";
import { withSlot } from "@/lib/slotdb";

export const dynamic = "force-dynamic";

// Largest attachment this route gives
const MAX_ATTACHMENT = 20e6;

const integer = (v: string | null, min: number, max: number): number | null => {
  if (v === null || !/^\d+$/.test(v)) return null;
  const n = Number(v);
  return Number.isSafeInteger(n) && n >= min && n <= max ? n : null;
};

const refused = (status: number, error: string) => Response.json({ error }, { status });

// The bytes of one image or file of a saved Teams message of the administrator of .env, for Everflow through the
// communication service (OE-Copilot mcp, /everflow/teams/attachment). Bearer MCP_TOKEN only, like /api/everflow/saved.
// ?slot=<n>&chat=<chat as saved>&mid=<numeric id>&index=<1-based, images first, then files, as get_attachment>
// 200 the bytes; 404 {"error"} no such account, message or index; 409 {"error"} the attachment cannot be given now
// (light account, download still queued, image not saved yet...); 413 {"error"} over 20 MB.
export const GET = route(async (req) => {
  if (!config.mcpToken) throw new HttpError(404, "Not found");
  if (req.headers.has("origin")) throw new HttpError(403, "Requests from web pages are not accepted");
  if (!tokenMatches(req.headers.get("authorization"))) throw new HttpError(401, "Missing or wrong token", { "WWW-Authenticate": 'Bearer realm="teamsrelay"' });
  const query = new URL(req.url).searchParams;
  const slot = integer(query.get("slot"), 0, 0xffffffff);
  const mid = query.get("mid") ?? "";
  const index = integer(query.get("index"), 1, 1000);
  const chat = query.get("chat") ?? "";
  if (slot === null || index === null || !/^\d{1,20}$/.test(mid) || !chat || chat.length > 400) {
    throw new HttpError(400, "slot, chat (at most 400 characters), mid (digits) and index (1 to 1000) are required");
  }
  const userId = mcpUserId();
  if (!userId) throw new HttpError(503, "The administrator of .env does not exist");
  if (!slotsOf(appDb(), userId).some((s) => s.slot === slot && !s.stopped)) return refused(404, "No active Teams account with this slot");
  const m = withSlot(slot, (r) => r.messages(chat).find((x) => x.mid === mid));
  if (!m) return refused(404, "No saved message with this id in this chat");
  const all = attachmentsOf(m);
  if (index > all.length) return refused(404, `index out of range: the message has ${all.length} attachment(s)`);
  const x = all[index - 1];
  const got = await fetchAttachment(slot, x, MAX_ATTACHMENT);
  if ("note" in got) return refused(got.tooLarge ? 413 : 409, got.note);
  const name = x.kind === "file" ? x.name : x.f || `image-${index}${MEDIA_EXT[got.type] ? `.${MEDIA_EXT[got.type]}` : ""}`;
  return new Response(new Uint8Array(got.data), {
    headers: {
      "Content-Type": got.type || "application/octet-stream",
      "Content-Length": String(got.data.length),
      "Content-Disposition": inlineDisposition(name),
      "Cache-Control": "private, max-age=86400",
      "X-Content-Type-Options": "nosniff",
    },
  });
});
