"use client";
import { useCallback, useEffect, useState, useRef } from "react";
import { AccountPanel, type AccountRow, type Capability } from "./AccountPanel";
import { ConnectForm, SignInProgress, type SignIn } from "./ConnectForm";
export type Snapshot = {
  rows: AccountRow[];
  commands?: { account: string; error?: string }[];
  signins?: SignIn[];
};
export type AccountTransport = {
  load: () => Promise<Snapshot>;
  toggle: (account: string, cap: Capability, on: boolean) => Promise<unknown>;
  remove: (account: string) => Promise<unknown>;
  // Teams calls on or off for an account without a browser
  setCalls?: (account: string, on: boolean) => Promise<unknown>;
  add: () => void;
  connect?: (account: string, capabilities: Capability[]) => Promise<unknown>;
  connected?: () => void;
};
export function Management({
  transport,
  initialAccount,
}: {
  transport: AccountTransport;
  initialAccount?: string;
}) {
  const current = useRef(transport);
  useEffect(() => {
    current.current = transport;
  }, [transport]);
  const [snapshot, setSnapshot] = useState<Snapshot>({ rows: [] });
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState("");
  const [error, setError] = useState("");
  const [feedback, setFeedback] = useState("");
  const [connecting, setConnecting] = useState<string | null>(
    initialAccount ?? null,
  );
  const completed = useRef(new Set<string>());
  const load = useCallback(async () => {
    try {
      const next = await current.current.load();
      setSnapshot(next);
      const fresh =
        next.signins?.filter(
          (j) => j.phase === "complete" && !completed.current.has(j.id),
        ) ?? [];
      for (const job of fresh) completed.current.add(job.id);
      if (fresh.length) current.current.connected?.();
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
  async function action(
    account: string,
    fn: () => Promise<unknown>,
    done = "Request accepted. Pending Teams operations apply on the next sync.",
  ) {
    setPending(account);
    setError("");
    setFeedback("");
    try {
      await fn();
      await load();
      setFeedback(done);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPending("");
    }
  }
  const queued = snapshot.commands?.[0]?.account;
  const rows: AccountRow[] = [
    ...snapshot.rows,
    ...(snapshot.commands ?? [])
      .filter((c) => !snapshot.rows.some((r) => r.account === c.account))
      .map((c) => ({
        account: c.account,
        capabilities: [],
        status: c.error ? "Operation failed" : "Removal pending",
        message: c.error,
      })),
  ];
  const open = (snapshot.signins ?? []).filter((j) => j.phase !== "complete");
  const onConnect = (account: string, caps: Capability[]) =>
    void action(account, () => transport.connect!(account, caps));
  // the form opens under the row it was asked from; "Add account" opens it at the top
  const expanded =
    connecting && rows.some((r) => r.account === connecting)
      ? connecting
      : open.find((j) => rows.some((r) => r.account === j.account))?.account;
  const atTop = transport.connect && connecting !== null && !(connecting && expanded === connecting);
  const orphan = open.filter((j) => !rows.some((r) => r.account === j.account));
  const close = () => setConnecting(null);
  return (
    <AccountPanel
      rows={rows}
      expanded={expanded}
      inline={
        transport.connect && expanded ? (
          connecting === expanded ? (
            <ConnectForm
              key={expanded}
              initial={expanded}
              pending={!!pending}
              signins={open.filter((j) => j.account === expanded)}
              onConnect={onConnect}
              onClose={close}
            />
          ) : (
            <div className="space-y-1">
              <SignInProgress
                signins={open.filter((j) => j.account === expanded)}
                onConnect={onConnect}
              />
            </div>
          )
        ) : undefined
      }
      loading={loading}
      pending={pending || queued}
      error={error}
      feedback={queued ? "Applying account operation..." : feedback}
      busyAccounts={snapshot.signins
        ?.filter((j) => j.phase === "queued" || j.phase === "browser")
        .map((j) => j.account)}
      onReconnect={
        transport.connect
          ? (account) => setConnecting(connecting === account ? null : account)
          : undefined
      }
      onAdd={() => {
        if (transport.connect) {
          setConnecting("");
          return;
        }
        try {
          transport.add();
        } catch (e) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }}
      onCalls={
        transport.setCalls
          ? (a, on) =>
              void action(
                a,
                () => transport.setCalls!(a, on),
                `Calls ${on ? "on" : "off"} for ${a}. Applies within a minute.`,
              )
          : undefined
      }
      onToggle={(a, c, on) => void action(a, () => transport.toggle(a, c, on))}
      onRemove={(a) => void action(a, () => transport.remove(a))}
    >
      {(atTop || (transport.connect && orphan.length > 0)) && (
        <div className="space-y-1 border-b p-2">
          {atTop ? (
            <ConnectForm
              key={connecting ?? ""}
              initial={connecting ?? ""}
              pending={!!pending}
              signins={orphan}
              onConnect={onConnect}
              onClose={close}
            />
          ) : (
            <SignInProgress signins={orphan} onConnect={onConnect} />
          )}
        </div>
      )}
    </AccountPanel>
  );
}
