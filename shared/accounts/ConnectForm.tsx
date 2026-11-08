"use client";
import { useState } from "react";
import type { Capability } from "./AccountPanel";
export type SignIn = {
  id: string;
  account: string;
  capabilities: Capability[];
  phase: string;
  url?: string;
  error?: string;
  route?: string;
};
export function ConnectForm({
  onConnect,
  signins = [],
  initial = "",
  pending = false,
}: {
  onConnect: (account: string, capabilities: Capability[]) => void;
  signins?: SignIn[];
  initial?: string;
  pending?: boolean;
}) {
  const [account, setAccount] = useState(initial);
  const [caps, setCaps] = useState<Capability[]>(["mail", "calendar", "chat"]);
  return (
    <div className="space-y-4 border-b bg-muted/30 p-4">
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          onConnect(account.trim().toLowerCase(), caps);
        }}
      >
        <label className="flex flex-col gap-1 text-sm font-medium">
          Microsoft account
          <input
            type="email"
            required
            autoComplete="email"
            value={account}
            onChange={(e) => setAccount(e.target.value)}
            placeholder="you@company.com"
            className="min-h-11 rounded-lg border bg-background px-3 font-normal focus-visible:outline-2 focus-visible:outline-ring"
          />
        </label>
        <fieldset className="flex flex-wrap gap-4">
          <legend className="mb-1 text-sm">Services to connect</legend>
          {(["mail", "calendar", "chat"] as const).map((cap) => (
            <label
              key={cap}
              className="flex min-h-11 items-center gap-2 text-sm"
            >
              <input
                type="checkbox"
                checked={caps.includes(cap)}
                onChange={(e) =>
                  setCaps(
                    e.target.checked
                      ? [...caps, cap]
                      : caps.filter((c) => c !== cap),
                  )
                }
              />
              {cap === "chat" ? "Teams" : cap === "mail" ? "Mail" : "Calendar"}
            </label>
          ))}
        </fieldset>
        <p className="text-sm text-muted-foreground">
          Existing services stay connected. Use the account switches to turn
          them off. Saved sessions are reused first. If Microsoft requires
          interaction, open the sign-in link below once. The permitted login
          route is remembered.
        </p>
        <button
          type="submit"
          disabled={pending || !caps.length}
          className="min-h-11 rounded-lg bg-primary px-4 text-sm text-primary-foreground disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-ring"
        >
          {pending ? "Connecting..." : "Connect account"}
        </button>
      </form>
      {signins.map((j) => (
        <div
          key={j.id}
          className="rounded-lg border bg-background p-3 text-sm"
          aria-live="polite"
        >
          <p className="break-all font-medium">{j.account}</p>
          <p className="mt-1 text-muted-foreground">
            {j.phase === "queued"
              ? "Finding a reusable Microsoft session..."
              : j.phase === "browser"
                ? "Waiting for Microsoft sign-in. This page updates automatically."
                : j.phase === "complete"
                  ? `Connected${j.route ? ` using ${j.route}` : ""}.`
                  : "Sign-in needs attention."}
          </p>
          {j.error && (
            <p className="mt-1 text-destructive" role="alert">
              {j.error}
            </p>
          )}
          {j.url && j.phase !== "complete" && (
            <a
              className="mt-2 inline-flex min-h-11 items-center rounded-lg border px-3 focus-visible:outline-2 focus-visible:outline-ring"
              href={j.url}
              target="_blank"
              rel="noreferrer"
            >
              Open Microsoft sign-in
            </a>
          )}
          {j.phase === "blocked" &&
            !j.error?.includes("AADSTS53003") &&
            !j.error?.includes("cancelled") && (
              <button
                className="ml-2 min-h-11 rounded-lg border px-3"
                type="button"
                onClick={() => onConnect(j.account, j.capabilities)}
              >
                Resume connection
              </button>
            )}
        </div>
      ))}
    </div>
  );
}
