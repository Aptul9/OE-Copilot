import { HttpError } from "./http-error";

export type DeviceKind = "mobile" | "desktop";

// Phone or tablet, from what the browser says about itself. iPadOS reports itself as a Mac, with a touch screen.
export function deviceKindOf(userAgent: string, touchPoints: number): DeviceKind {
  return /Android|iPhone|iPad|iPod|Mobile/i.test(userAgent) || (/Macintosh/.test(userAgent) && touchPoints > 1) ? "mobile" : "desktop";
}

// A Web Push subscription as the browser gives it (PushSubscription.toJSON()), checked the same way by the web app and
// the API of the local relay; kept as JSON of its endpoint, keys and kind (the device it was made on, `device` of the
// body: desktop unless the browser said mobile)
export function subscriptionOf(b: Record<string, unknown>): { endpoint: string; json: string } {
  const { endpoint, keys } = b as { endpoint?: unknown; keys?: { p256dh?: unknown; auth?: unknown } };
  if (typeof endpoint !== "string" || !endpoint.startsWith("https://") || endpoint.length > 2000) throw new HttpError(400, "Invalid subscription endpoint");
  const b64 = /^[A-Za-z0-9_-]{16,200}=*$/;
  if (typeof keys?.p256dh !== "string" || typeof keys.auth !== "string" || !b64.test(keys.p256dh) || !b64.test(keys.auth)) {
    throw new HttpError(400, "Invalid subscription keys");
  }
  const kind: DeviceKind = b.device === "mobile" ? "mobile" : "desktop";
  return { endpoint, json: JSON.stringify({ endpoint, keys: { p256dh: keys.p256dh, auth: keys.auth }, kind }) };
}
