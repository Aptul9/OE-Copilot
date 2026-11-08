import { z } from "zod";
import { ConfigError, issuesText } from "@/shared/env";

// Shared Microsoft broker of the light agents: the OE calendar agent hands out resource tokens of a unified account
// (/api/token-resource). Each slot maps to its own account and chat service region; a slot not listed keeps its own
// browser sign-in. The supervisor never passes a broker setting to a slot that is not listed.
export type BrokerSlot = { account: string; region: string };
export type SharedBroker = { url: string; key: string; slots: Record<string, BrokerSlot> };

// the chat service regions of Teams are short lowercase words (amer, emea, apac, in, au, ...)
export const REGION = /^[a-z][a-z0-9]{1,15}$/;

const Slots = z
  .record(
    z.string().regex(/^[1-9][0-9]{0,3}$/, "slot numbers start at 1"),
    z.object({ account: z.email().transform((a) => a.toLowerCase()), region: z.string().regex(REGION, "not a chat service region") }).strict(),
  )
  .refine((m) => Object.keys(m).length > 0, "no slot listed");

export function parseBrokerSlots(text: string): Record<string, BrokerSlot> {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    throw new ConfigError("SHARED_BROKER_SLOTS is not valid JSON");
  }
  const r = Slots.safeParse(raw);
  if (!r.success) throw new ConfigError(`SHARED_BROKER_SLOTS: ${issuesText(r.error)}`);
  const seen = new Map<string, string>();
  for (const [slot, { account }] of Object.entries(r.data)) {
    // two slots on one account would push the same messages twice
    const other = seen.get(account);
    if (other) throw new ConfigError(`SHARED_BROKER_SLOTS: ${account} is mapped to slots ${other} and ${slot}`);
    seen.set(account, slot);
  }
  return r.data;
}

// The broker variables of the light agent of slot n: all four for a mapped slot, none otherwise
export function brokerEnv(n: number, broker: SharedBroker | undefined): Record<string, string> {
  const slot = broker?.slots[String(n)];
  if (!broker || !slot) return {};
  return { SHARED_BROKER_URL: broker.url, SHARED_BROKER_KEY: broker.key, SHARED_BROKER_ACCOUNT: slot.account, SHARED_BROKER_REGION: slot.region };
}
