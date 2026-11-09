import diagnostics from "node:diagnostics_channel";
import { errorText, log } from "../log";

type Request = { origin?: unknown; path?: string; method?: string };
type Seen = { at: number; sent: number; answer: number };
type Message = { request: Request; error?: unknown };

// Requests of the light agents that ended, or failed, after `ms`: when their headers went out and when the answer came,
// which tells a request that never left (queued, connecting) from one the server kept. Returns the unsubscribe.
export function traceSlowRequests(ms = 10_000): () => void {
  const seen = new WeakMap<object, Seen>();
  const report = (r: Request, error?: unknown) => {
    const s = seen.get(r);
    if (!s) return;
    seen.delete(r);
    const took = Date.now() - s.at;
    if (took < ms) return;
    const since = (t: number) => (t ? `${t - s.at}ms` : "none");
    const failed = error === undefined ? "" : ` error=${errorText(error)}`;
    log.warn("fetch", `slow ${r.method} ${String(r.origin)}${(r.path ?? "").split("?")[0]} after ${took} ms sent=${since(s.sent)} answer=${since(s.answer)}${failed}`);
  };
  const mark = (r: Request, k: "sent" | "answer") => {
    const s = seen.get(r);
    if (s && !s[k]) s[k] = Date.now();
  };
  const handlers: Record<string, (m: Message) => void> = {
    "undici:request:create": (m) => void seen.set(m.request, { at: Date.now(), sent: 0, answer: 0 }),
    "undici:client:sendHeaders": (m) => mark(m.request, "sent"),
    "undici:request:headers": (m) => mark(m.request, "answer"),
    "undici:request:trailers": (m) => report(m.request),
    "undici:request:error": (m) => report(m.request, m.error),
  };
  const subscribed = Object.entries(handlers).map(([name, f]) => {
    const listener = (m: unknown) => f(m as Message);
    diagnostics.subscribe(name, listener);
    return () => diagnostics.unsubscribe(name, listener);
  });
  return () => subscribed.forEach((off) => off());
}
