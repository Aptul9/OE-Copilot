"use client";
import { useEffect, useRef, useState } from "react";
import {
  CheckIcon,
  ExternalLinkIcon,
  LoaderCircleIcon,
  PlugZapIcon,
  RotateCcwIcon,
  TriangleAlertIcon,
  XIcon,
} from "lucide-react";
import { ICON_BUTTON, SERVICES, focus, type Capability } from "./AccountPanel";
export type SignIn = {
  id: string;
  account: string;
  capabilities: Capability[];
  phase: string;
  url?: string;
  error?: string;
  route?: string;
  slot?: number;
  ended?: number;
};
const SMALL_BUTTON = `inline-flex h-8 items-center gap-1.5 rounded-lg px-3 text-xs font-medium transition-colors pointer-coarse:h-11 disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none ${focus}`;
export function SignInProgress({
  signins,
  onConnect,
  onCancel,
}: {
  signins: SignIn[];
  onConnect: (account: string, capabilities: Capability[]) => void;
  // stops a running sign-in, or takes a stopped one off the list
  onCancel?: (signin: SignIn) => void;
}) {
  return (
    <>
      {signins.map((j) => (
        <div
          key={j.id}
          className="flex flex-wrap items-center gap-2 rounded-lg border bg-background px-2.5 py-1.5 text-xs"
          aria-live="polite"
        >
          <span
            aria-hidden="true"
            className={
              j.phase === "complete"
                ? "text-success"
                : j.phase === "blocked"
                  ? "text-destructive"
                  : "text-primary"
            }
          >
            {j.phase === "complete" ? (
              <CheckIcon className="size-3.5" />
            ) : j.phase === "blocked" ? (
              <TriangleAlertIcon className="size-3.5" />
            ) : (
              <LoaderCircleIcon className="size-3.5 animate-spin motion-reduce:animate-none" />
            )}
          </span>
          <span className="min-w-0 flex-1">
            <span className="font-medium">{j.account}</span>{" "}
            <span className="text-muted-foreground">
              {j.phase === "queued"
                ? "Finding a reusable Microsoft session..."
                : j.phase === "browser"
                  ? "Waiting for Microsoft sign-in. This page updates automatically."
                  : j.phase === "complete"
                    ? `Connected${j.route ? ` using ${j.route}` : ""}.`
                    : "Sign-in needs attention."}
            </span>
            {j.error && (
              <span className="block text-destructive" role="alert">
                {j.error}
              </span>
            )}
          </span>
          {j.url && j.phase !== "complete" && (
            <a
              className={`${SMALL_BUTTON} border hover:bg-accent`}
              href={j.url}
              target="_blank"
              rel="noreferrer"
            >
              <ExternalLinkIcon aria-hidden="true" className="size-3.5" />
              Open Microsoft sign-in
            </a>
          )}
          {j.phase === "blocked" &&
            !j.error?.includes("AADSTS53003") &&
            !j.error?.includes("cancelled") && (
              <button
                className={`${SMALL_BUTTON} border hover:bg-accent`}
                type="button"
                onClick={() => onConnect(j.account, j.capabilities)}
              >
                <RotateCcwIcon aria-hidden="true" className="size-3.5" />
                Resume connection
              </button>
            )}
          {onCancel && j.phase !== "complete" && (
            <button
              className={`${SMALL_BUTTON} border text-muted-foreground hover:bg-destructive/10 hover:text-destructive`}
              type="button"
              aria-label={`${j.phase === "blocked" ? "Dismiss" : "Cancel"} the sign-in of ${j.account}`}
              onClick={() => onCancel(j)}
            >
              <XIcon aria-hidden="true" className="size-3.5" />
              {j.phase === "blocked" ? "Dismiss" : "Cancel"}
            </button>
          )}
        </div>
      ))}
    </>
  );
}
export function ConnectForm({
  onConnect,
  onCancel,
  onClose,
  signins = [],
  initial = "",
  pending = false,
}: {
  onConnect: (account: string, capabilities: Capability[]) => void;
  onCancel?: (signin: SignIn) => void;
  onClose?: () => void;
  signins?: SignIn[];
  initial?: string;
  pending?: boolean;
}) {
  const [account, setAccount] = useState(initial);
  const [caps, setCaps] = useState<Capability[]>(["mail", "calendar", "chat"]);
  const box = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const submit = useRef<HTMLButtonElement>(null);
  // opened from a row far down the list: bring it into view and put the cursor where the next action is
  useEffect(() => {
    const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    box.current?.scrollIntoView?.({ block: "nearest", behavior: reduce ? "auto" : "smooth" });
    (initial ? submit.current : input.current)?.focus({ preventScroll: true });
  }, [initial]);
  return (
    <div ref={box} className="space-y-2 rounded-lg border bg-muted/30 p-2.5">
      <form
        className="flex flex-wrap items-center gap-2"
        aria-label={initial ? `Connect services of ${initial}` : "Add a Microsoft account"}
        onSubmit={(e) => {
          e.preventDefault();
          onConnect(account.trim().toLowerCase(), caps);
        }}
      >
        <label className="min-w-48 flex-1">
          <span className="sr-only">Microsoft account</span>
          <input
            ref={input}
            type="email"
            required
            autoComplete="email"
            value={account}
            onChange={(e) => setAccount(e.target.value)}
            placeholder="you@company.com"
            className={`h-8 w-full rounded-lg border bg-background px-2.5 text-sm pointer-coarse:h-11 ${focus}`}
          />
        </label>
        <fieldset className="flex items-center gap-1">
          <legend className="sr-only">Services to connect</legend>
          {SERVICES.map(({ cap, label, Icon }) => (
            <label
              key={cap}
              title={label}
              className={`${ICON_BUTTON} cursor-pointer border text-muted-foreground hover:bg-accent has-[:checked]:border-primary has-[:checked]:bg-primary has-[:checked]:text-primary-foreground has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-ring`}
            >
              <input
                type="checkbox"
                className="sr-only"
                checked={caps.includes(cap)}
                onChange={(e) =>
                  setCaps(
                    e.target.checked
                      ? [...caps, cap]
                      : caps.filter((c) => c !== cap),
                  )
                }
              />
              <Icon aria-hidden={true} className="size-4" />
              <span className="sr-only">{label}</span>
            </label>
          ))}
        </fieldset>
        <button
          ref={submit}
          type="submit"
          disabled={pending || !caps.length}
          className={`${SMALL_BUTTON} bg-primary text-primary-foreground hover:bg-primary/90`}
        >
          {pending ? (
            <LoaderCircleIcon
              aria-hidden="true"
              className="size-3.5 animate-spin motion-reduce:animate-none"
            />
          ) : (
            <PlugZapIcon aria-hidden="true" className="size-3.5" />
          )}
          {pending ? "Connecting..." : "Connect account"}
        </button>
        {onClose && (
          <button
            type="button"
            className={`${ICON_BUTTON} text-muted-foreground hover:bg-accent`}
            aria-label="Close"
            title="Close"
            onClick={onClose}
          >
            <XIcon aria-hidden="true" className="size-4" />
          </button>
        )}
      </form>
      <p className="text-xs text-muted-foreground">
        Saved sessions are reused first. If Microsoft asks, open the sign-in
        link that appears here once. Existing services stay connected.
      </p>
      <SignInProgress signins={signins} onConnect={onConnect} onCancel={onCancel} />
    </div>
  );
}
