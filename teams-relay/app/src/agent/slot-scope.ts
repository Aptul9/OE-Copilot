import { AsyncLocalStorage } from "node:async_hooks";

// The light slot the current work belongs to, in the light host where every slot shares one process (src/agent/light/host.ts).
// Outside any scope (one slot per process, tests) the slot is 0.
const scope = new AsyncLocalStorage<number>();

export const inSlot = <T>(slot: number, fn: () => T): T => scope.run(slot, fn);
export const currentSlot = () => scope.getStore() ?? 0;
export const slotPrefix = () => {
  const s = scope.getStore();
  return s === undefined ? "" : `[${s}] `;
};

// Module state that belongs to one account: one instance per slot
export function perSlot<T>(make: () => T): () => T {
  const all = new Map<number, T>();
  return () => {
    const slot = currentSlot();
    let v = all.get(slot);
    if (v === undefined) all.set(slot, (v = make()));
    return v;
  };
}
