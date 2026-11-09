"use client";
import { useCallback, useEffect, useState, useRef, type ReactNode } from "react";
import { AccountPanel, type AccountRow, type Capability } from "./AccountPanel";
import { ConnectForm, SignInProgress, type SignIn } from "./ConnectForm";
export type Snapshot = {
  rows: AccountRow[];
  commands?: { account: string; operation?: string; on?: boolean; error?: string }[];
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
  renderLogo,
}: {
  transport: AccountTransport;
  initialAccount?: string;
  renderLogo?: (row: AccountRow) => ReactNode;
}) {
  const current = useRef(transport);
  useEffect(() => {
    current.current = transport;
  }, [transport]);
  const [snapshot, setSnapshot] = useState<Snapshot>({ rows: [] });
  const [loading, setLoading] = useState(true);
  // the changes on their way, "account|switch" to the state asked: each locks only its own switch, the others stay usable
  const [inflight, setInflight] = useState<Record<string, boolean>>({});
  const [error, setError] = useState("");
  const [feedback, setFeedback] = useState("");
  const [connecting, setConnecting] = useState<string | null>(
    initialAccount ?? null,
  );
  const completed = useRef(new Set<string>());
  // reads overlap while changes run: only the latest one asked is shown
  const asked = useRef(0);
  const chains = useRef(new Map<string, Promise<void>>());
  const load = useCallback(async () => {
    const n = ++asked.current;
    try {
      const next = await current.current.load();
      if (n !== asked.current) return;
      setSnapshot(next);
      const fresh =
        next.signins?.filter(
          (j) => j.phase === "complete" && !completed.current.has(j.id),
        ) ?? [];
      for (const job of fresh) completed.current.add(job.id);
      if (fresh.length) current.current.connected?.();
      const failed = next.commands?.find((c) => c.error);
      setError(failed ? `${failed.account}: ${failed.error}` : "");
    } catch (e) {
      if (n === asked.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 15000);
    return () => clearInterval(timer);
  }, [load]);
  const mark = (key: string, value?: boolean) =>
    setInflight((m) => {
      const next = { ...m };
      if (value === undefined) delete next[key];
      else next[key] = value;
      return next;
    });
  async function action(
    account: string,
    what: Capability | "calls" | "remove" | "connect",
    target: boolean,
    fn: () => Promise<unknown>,
    done = "Request accepted. Pending Teams operations apply on the next sync.",
  ) {
    const key = `${account}|${what}`;
    mark(key, target);
    setError("");
    // accounts change in parallel; one account's changes run one after the other, since each writes its whole list of services
    const before = chains.current.get(account) ?? Promise.resolve();
    const run = before.then(async () => {
      try {
        await fn();
        setFeedback(done);
        await load();
      } catch (e) {
        setError(`${account}: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        mark(key);
      }
    });
    chains.current.set(account, run);
    await run;
    if (chains.current.get(account) === run) chains.current.delete(account);
  }
  const commands = snapshot.commands ?? [];
  // a queued agent command locks its own switch (or the whole row for a removal) until TeamsRelay applies it
  const busy = [
    ...Object.keys(inflight).map((k) => k.replace(/\|remove$/, "|*")),
    ...commands
      .filter((c) => !c.error)
      .map((c) => `${c.account}|${c.operation === "chat" || c.operation === "calls" ? c.operation : "*"}`),
  ];
  const shown = (row: AccountRow): AccountRow => {
    const caps = new Set(row.capabilities);
    let calls = row.calls;
    const want = (what: string, on: boolean) => {
      if (what === "calls") calls = calls === undefined ? undefined : on;
      else if (what === "mail" || what === "calendar" || what === "chat") {
        if (on) caps.add(what);
        else caps.delete(what);
      }
    };
    for (const c of commands)
      if (c.account === row.account && !c.error && typeof c.on === "boolean")
        want(c.operation ?? "", c.on);
    for (const [k, on] of Object.entries(inflight)) {
      const cut = k.lastIndexOf("|");
      if (k.slice(0, cut) === row.account) want(k.slice(cut + 1), on);
    }
    return { ...row, capabilities: [...caps], calls };
  };
  const rows: AccountRow[] = [
    ...snapshot.rows.map(shown),
    ...commands
      .filter((c) => !snapshot.rows.some((r) => r.account === c.account))
      .map((c) => ({
        account: c.account,
        capabilities: [],
        status: c.error ? "Operation failed" : "Removal pending",
        message: c.error,
      })),
  ];
  const open = (snapshot.signins ?? []).filter((j) => j.phase !== "complete");
  const connectingNow = (account: string) => `${account}|connect` in inflight;
  const onConnect = (account: string, caps: Capability[]) =>
    void action(account, "connect", true, () => transport.connect!(account, caps));
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
      renderLogo={renderLogo}
      expanded={expanded}
      inline={
        transport.connect && expanded ? (
          connecting === expanded ? (
            <ConnectForm
              key={expanded}
              initial={expanded}
              pending={connectingNow(expanded)}
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
      busy={busy}
      error={error}
      feedback={feedback}
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
                "calls",
                on,
                () => transport.setCalls!(a, on),
                `Calls ${on ? "on" : "off"} for ${a}. Applies within a minute.`,
              )
          : undefined
      }
      onToggle={(a, c, on) =>
        void action(a, c, on, () => transport.toggle(a, c, on), `${a}: saved.`)
      }
      onRemove={(a) => void action(a, "remove", true, () => transport.remove(a))}
    >
      {(atTop || (transport.connect && orphan.length > 0)) && (
        <div className="space-y-1 border-b p-2">
          {atTop ? (
            <ConnectForm
              key={connecting ?? ""}
              initial={connecting ?? ""}
              pending={Object.keys(inflight).some(
                (k) =>
                  k.endsWith("|connect") &&
                  !rows.some((r) => k === `${r.account}|connect`),
              )}
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
