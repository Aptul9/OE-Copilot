"use client";
import { useState } from "react";
import {
  CheckIcon,
  ExternalLinkIcon,
  LoaderCircleIcon,
  PlugZapIcon,
  RotateCcwIcon,
  TriangleAlertIcon,
} from "lucide-react";
import { SERVICES, type Capability } from "./AccountPanel";
export type SignIn = {
  id: string;
  account: string;
  capabilities: Capability[];
  phase: string;
  url?: string;
  error?: string;
  route?: string;
};
const focus =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
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
    <div className="space-y-4 border-b bg-muted/30 p-4 sm:p-5">
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          onConnect(account.trim().toLowerCase(), caps);
        }}
      >
        <label className="flex flex-col gap-1.5 text-sm font-medium">
          Microsoft account
          <input
            type="email"
            required
            autoComplete="email"
            value={account}
            onChange={(e) => setAccount(e.target.value)}
            placeholder="you@company.com"
            className={`min-h-11 rounded-lg border bg-background px-3 font-normal ${focus}`}
          />
        </label>
        <fieldset>
          <legend className="mb-2 text-sm font-medium">
            Services to connect
          </legend>
          <div className="grid gap-2 sm:grid-cols-3">
            {SERVICES.map(({ cap, label, Icon }) => (
              <label
                key={cap}
                className="flex min-h-14 cursor-pointer items-center gap-3 rounded-xl border bg-background p-2.5 transition-colors hover:bg-accent has-[:checked]:border-primary/40 has-[:checked]:bg-primary/5 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-ring motion-reduce:transition-none"
              >
                <input
                  type="checkbox"
                  className="peer sr-only"
                  checked={caps.includes(cap)}
                  onChange={(e) =>
                    setCaps(
                      e.target.checked
                        ? [...caps, cap]
                        : caps.filter((c) => c !== cap),
                    )
                  }
                />
                <span
                  aria-hidden="true"
                  className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground transition-colors peer-checked:bg-primary peer-checked:text-primary-foreground motion-reduce:transition-none"
                >
                  <Icon aria-hidden={true} className="size-4" />
                </span>
                <span className="min-w-0 flex-1 text-sm font-medium">
                  {label}
                </span>
                <span
                  aria-hidden="true"
                  className="flex size-5 shrink-0 items-center justify-center rounded-md border bg-background text-transparent transition-colors peer-checked:border-primary peer-checked:bg-primary peer-checked:text-primary-foreground motion-reduce:transition-none"
                >
                  <CheckIcon className="size-3.5" />
                </span>
              </label>
            ))}
          </div>
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
          className={`inline-flex min-h-11 items-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none ${focus}`}
        >
          {pending ? (
            <LoaderCircleIcon
              aria-hidden="true"
              className="size-4 animate-spin motion-reduce:animate-none"
            />
          ) : (
            <PlugZapIcon aria-hidden="true" className="size-4" />
          )}
          {pending ? "Connecting..." : "Connect account"}
        </button>
      </form>
      {signins.map((j) => (
        <div
          key={j.id}
          className="flex items-start gap-3 rounded-xl border bg-background p-3 text-sm"
          aria-live="polite"
        >
          <span
            aria-hidden="true"
            className={`flex size-9 shrink-0 items-center justify-center rounded-lg ${j.phase === "complete" ? "bg-success/15 text-success" : j.phase === "blocked" ? "bg-destructive/10 text-destructive" : "bg-primary/10 text-primary"}`}
          >
            {j.phase === "complete" ? (
              <CheckIcon className="size-4" />
            ) : j.phase === "blocked" ? (
              <TriangleAlertIcon className="size-4" />
            ) : (
              <LoaderCircleIcon className="size-4 animate-spin motion-reduce:animate-none" />
            )}
          </span>
          <div className="min-w-0 flex-1">
            <p className="break-all font-medium">{j.account}</p>
            <p className="mt-0.5 text-muted-foreground">
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
            <div className="mt-2 flex flex-wrap gap-2 empty:hidden">
              {j.url && j.phase !== "complete" && (
                <a
                  className={`inline-flex min-h-11 items-center gap-2 rounded-lg border px-3 transition-colors hover:bg-accent motion-reduce:transition-none ${focus}`}
                  href={j.url}
                  target="_blank"
                  rel="noreferrer"
                >
                  <ExternalLinkIcon aria-hidden="true" className="size-4" />
                  Open Microsoft sign-in
                </a>
              )}
              {j.phase === "blocked" &&
                !j.error?.includes("AADSTS53003") &&
                !j.error?.includes("cancelled") && (
                  <button
                    className={`inline-flex min-h-11 items-center gap-2 rounded-lg border px-3 transition-colors hover:bg-accent motion-reduce:transition-none ${focus}`}
                    type="button"
                    onClick={() => onConnect(j.account, j.capabilities)}
                  >
                    <RotateCcwIcon aria-hidden="true" className="size-4" />
                    Resume connection
                  </button>
                )}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}
