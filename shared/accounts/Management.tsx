"use client";
import { useCallback, useEffect, useState, useRef } from "react";
import { AccountPanel, type AccountRow, type Capability } from "./AccountPanel";
export type Snapshot = {
  rows: AccountRow[];
  commands?: { account: string; error?: string }[];
};
export type AccountTransport = {
  load: () => Promise<Snapshot>;
  toggle: (account: string, cap: Capability, on: boolean) => Promise<unknown>;
  remove: (account: string) => Promise<unknown>;
  add: () => void;
};
export function Management({ transport }: { transport: AccountTransport }) {
  const current = useRef(transport);
  current.current = transport;
  const [snapshot, setSnapshot] = useState<Snapshot>({ rows: [] });
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState("");
  const [error, setError] = useState("");
  const [feedback, setFeedback] = useState("");
  const load = useCallback(async () => {
    try {
      const next = await current.current.load();
      setSnapshot(next);
      setError(next.commands?.find((c) => c.error)?.error ?? "");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 15000);
    return () => clearInterval(timer);
  }, [load]);
  async function action(account: string, fn: () => Promise<unknown>) {
    setPending(account);
    setError("");
    setFeedback("");
    try {
      await fn();
      await load();
      setFeedback(
        "Request accepted. Pending Teams operations apply on the next sync.",
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending("");
    }
  }
  const queued = snapshot.commands?.[0]?.account;
  return (
    <AccountPanel
      rows={[
        ...snapshot.rows,
        ...(snapshot.commands ?? [])
          .filter((c) => !snapshot.rows.some((r) => r.account === c.account))
          .map((c) => ({
            account: c.account,
            capabilities: [],
            status: c.error ? "Operation failed" : "Removal pending",
            message: c.error,
          })),
      ]}
      loading={loading}
      pending={pending || queued}
      error={error}
      feedback={queued ? "Applying account operation..." : feedback}
      onAdd={() => {
        try {
          transport.add();
        } catch (e) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }}
      onToggle={(a, c, on) => void action(a, () => transport.toggle(a, c, on))}
      onRemove={(a) => void action(a, () => transport.remove(a))}
    />
  );
}
