"use client";
import { useState } from "react";
import type { ReactNode } from "react";
export type Capability = "mail" | "calendar" | "chat";
export type AccountRow = {
  account: string;
  capabilities: Capability[];
  status: string;
  message?: string;
  unavailable?: Partial<Record<Capability, string>>;
};
export function AccountPanel({
  rows,
  pending,
  error,
  onToggle,
  onRemove,
  onAdd,
  loading,
  feedback,
  children,
  onReconnect,
  busyAccounts = [],
}: {
  rows: AccountRow[];
  pending?: string;
  error?: string;
  onToggle: (account: string, cap: Capability, on: boolean) => void;
  onRemove?: (account: string) => void;
  onAdd?: () => void;
  loading?: boolean;
  feedback?: string;
  children?: ReactNode;
  onReconnect?: (account: string) => void;
  busyAccounts?: string[];
}) {
  const [confirm, setConfirm] = useState("");
  return (
    <section
      className="rounded-xl border bg-card text-card-foreground"
      data-account-panel
      aria-label="Microsoft accounts"
    >
      <header className="border-b p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-base font-semibold">Microsoft accounts</h2>
          <button
            type="button"
            className="min-h-11 rounded-lg border px-3 text-sm hover:bg-accent"
            disabled={!!pending || loading}
            onClick={onAdd}
          >
            Add account
          </button>
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          Mail, calendar and Teams. Same accounts in both apps.
        </p>
      </header>
      {children}
      {error && (
        <p role="alert" className="p-4 text-sm text-destructive">
          {error}
        </p>
      )}
      {loading && (
        <p role="status" className="p-4">
          Loading accounts...
        </p>
      )}
      {feedback && (
        <p role="status" className="p-4 text-sm">
          {feedback}
        </p>
      )}
      {!loading && !rows.length && (
        <p className="p-4 text-sm text-muted-foreground">
          No accounts connected.
        </p>
      )}
      <ul className="divide-y">
        {rows.map((row) => (
          <li
            key={row.account}
            data-account={row.account}
            aria-busy={pending === row.account}
            className="flex flex-wrap items-center gap-4 p-4"
          >
            <div className="min-w-0 flex-1 basis-56">
              <p className="break-all text-sm font-medium">{row.account}</p>
              <p role="status" className="mt-1 text-xs text-muted-foreground">
                {pending === row.account
                  ? "Saving..."
                  : busyAccounts.includes(row.account)
                    ? "Connecting services..."
                    : row.status}
              </p>
              {row.message && (
                <p className="mt-1 text-xs text-destructive">{row.message}</p>
              )}
              {onReconnect && (
                <button
                  type="button"
                  className="mt-1 min-h-11 text-xs text-primary underline underline-offset-4"
                  onClick={() => onReconnect(row.account)}
                >
                  Connect services / sign in again
                </button>
              )}
            </div>
            <div className="flex flex-wrap gap-2">
              {(["mail", "calendar", "chat"] as const).map((cap) => {
                const on = row.capabilities.includes(cap);
                const label =
                  cap === "chat"
                    ? "Teams"
                    : cap === "mail"
                      ? "Mail"
                      : "Calendar";
                const reason = row.unavailable?.[cap];
                return (
                  <div key={cap}>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={on}
                      aria-label={`${label} of ${row.account}`}
                      title={reason}
                      disabled={
                        !!pending ||
                        !!reason ||
                        (busyAccounts.includes(row.account) && !on)
                      }
                      onClick={() => onToggle(row.account, cap, !on)}
                      className="flex min-h-11 items-center gap-2 rounded-lg border px-3 text-sm hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      <span
                        aria-hidden="true"
                        className={`flex h-5 w-9 items-center rounded-full p-0.5 ${on ? "justify-end bg-primary" : "justify-start bg-muted-foreground/30"}`}
                      >
                        <span className="size-4 rounded-full bg-background shadow-sm" />
                      </span>
                      {label}
                    </button>
                    {reason && (
                      <p className="mt-1 max-w-36 text-xs text-muted-foreground">
                        {reason}
                      </p>
                    )}
                  </div>
                );
              })}
            </div>
            <button
              type="button"
              className="flex min-h-11 min-w-11 items-center justify-center rounded-lg border text-destructive hover:bg-destructive/10 focus-visible:outline-2 focus-visible:outline-ring"
              aria-label={`Remove ${row.account}`}
              title="Remove account"
              disabled={!!pending}
              onClick={() => setConfirm(row.account)}
            >
              <svg
                aria-hidden="true"
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.75"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M3 6h18M9 6V4h6v2M5 6l1 14h12l1-14M10 10v6M14 10v6" />
              </svg>
            </button>
            {confirm === row.account && (
              <div
                role="alertdialog"
                aria-modal="false"
                aria-label={"Remove " + row.account}
                className="basis-full rounded-lg border border-destructive p-4"
              >
                <p>
                  Remove {row.account}? Saved mail and calendar access, matching
                  Teams slots and their local data will be deleted from both
                  apps. Microsoft mailbox contents stay unchanged. This cannot
                  be undone; adding again requires sign-in.
                </p>
                <div className="mt-3 flex gap-2">
                  <button
                    type="button"
                    className="min-h-11 rounded-lg bg-destructive px-3 text-white"
                    disabled={!!pending}
                    onClick={() => onRemove?.(row.account)}
                  >
                    Confirm removal
                  </button>
                  <button
                    type="button"
                    className="min-h-11 rounded-lg border px-3"
                    disabled={!!pending}
                    onClick={() => setConfirm("")}
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
