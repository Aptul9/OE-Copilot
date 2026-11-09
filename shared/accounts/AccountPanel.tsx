"use client";
import { useState } from "react";
import type { ComponentType, ReactNode } from "react";
import {
  AtSignIcon,
  CalendarDaysIcon,
  CheckIcon,
  LoaderCircleIcon,
  MailIcon,
  MessagesSquareIcon,
  PhoneIcon,
  PhoneOffIcon,
  PlugZapIcon,
  PlusIcon,
  Trash2Icon,
  TriangleAlertIcon,
} from "lucide-react";
export type Capability = "mail" | "calendar" | "chat";
export type AccountRow = {
  account: string;
  capabilities: Capability[];
  status: string;
  message?: string;
  unavailable?: Partial<Record<Capability, string>>;
  // Teams calls of an account without a browser; undefined when calls cannot be switched for it
  calls?: boolean;
};
export const SERVICES: {
  cap: Capability;
  label: string;
  Icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
}[] = [
  { cap: "mail", label: "Mail", Icon: MailIcon },
  { cap: "calendar", label: "Calendar", Icon: CalendarDaysIcon },
  { cap: "chat", label: "Teams", Icon: MessagesSquareIcon },
];
type Tone = "ok" | "warn" | "off" | "info";
const DOT: Record<Tone, string> = {
  ok: "bg-success",
  warn: "bg-destructive",
  off: "bg-muted-foreground/50",
  info: "bg-primary",
};
function toneOf(status: string, message?: string): Tone {
  if (message || /sign-in needed|failed/i.test(status)) return "warn";
  if (/stopped|unknown/i.test(status)) return "off";
  if (/pending|starting|login/i.test(status)) return "info";
  return "ok";
}
const focus =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
function Switch({ on }: { on: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={`flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors motion-reduce:transition-none ${on ? "justify-end bg-primary" : "justify-start bg-muted-foreground/30"}`}
    >
      <span className="size-4 rounded-full bg-background shadow-sm" />
    </span>
  );
}
function IconBox({ on, children }: { on: boolean; children: ReactNode }) {
  return (
    <span
      aria-hidden="true"
      className={`flex size-9 shrink-0 items-center justify-center rounded-lg transition-colors motion-reduce:transition-none ${on ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
    >
      {children}
    </span>
  );
}
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
  onCalls,
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
  onCalls?: (account: string, on: boolean) => void;
  busyAccounts?: string[];
}) {
  const [confirm, setConfirm] = useState("");
  return (
    <section
      className="overflow-hidden rounded-2xl border bg-card text-card-foreground shadow-sm"
      data-account-panel
      aria-label="Microsoft accounts"
    >
      <header className="flex flex-wrap items-start justify-between gap-3 border-b p-4 sm:p-5">
        <div className="flex min-w-0 items-start gap-3">
          <span
            aria-hidden="true"
            className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
          >
            <AtSignIcon className="size-5" />
          </span>
          <div className="min-w-0">
            <h2 className="text-base font-semibold">Microsoft accounts</h2>
            <p className="mt-0.5 text-sm text-muted-foreground">
              Mail, calendar and Teams. Same accounts in both apps.
            </p>
          </div>
        </div>
        <button
          type="button"
          className={`inline-flex min-h-11 items-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground shadow-xs transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none ${focus}`}
          disabled={!!pending || loading}
          onClick={onAdd}
        >
          <PlusIcon aria-hidden="true" className="size-4" />
          Add account
        </button>
      </header>
      {children}
      {error && (
        <p
          role="alert"
          className="flex items-start gap-2 border-b bg-destructive/10 p-4 text-sm text-destructive"
        >
          <TriangleAlertIcon
            aria-hidden="true"
            className="mt-0.5 size-4 shrink-0"
          />
          {error}
        </p>
      )}
      {loading && (
        <p
          role="status"
          className="flex items-center gap-2 p-4 text-sm text-muted-foreground"
        >
          <LoaderCircleIcon
            aria-hidden="true"
            className="size-4 animate-spin motion-reduce:animate-none"
          />
          Loading accounts...
        </p>
      )}
      {feedback && (
        <p
          role="status"
          className="flex items-center gap-2 border-b bg-muted/40 px-4 py-3 text-sm"
        >
          <CheckIcon
            aria-hidden="true"
            className="size-4 shrink-0 text-success"
          />
          {feedback}
        </p>
      )}
      {!loading && !rows.length && (
        <div className="flex flex-col items-center gap-2 p-8 text-center">
          <IconBox on={false}>
            <AtSignIcon className="size-4" />
          </IconBox>
          <p className="text-sm font-medium">No accounts connected</p>
          <p className="text-sm text-muted-foreground">
            Add a Microsoft account to read its mail, calendar and Teams chats.
          </p>
        </div>
      )}
      <ul className="divide-y">
        {rows.map((row) => {
          const busy =
            pending === row.account || busyAccounts.includes(row.account);
          const status =
            pending === row.account
              ? "Saving..."
              : busyAccounts.includes(row.account)
                ? "Connecting services..."
                : row.status;
          const tone = busy ? "info" : toneOf(row.status, row.message);
          const teamsOn = row.capabilities.includes("chat");
          return (
            <li
              key={row.account}
              data-account={row.account}
              aria-busy={pending === row.account}
              className="p-4 sm:p-5"
            >
              <div className="flex items-start gap-3">
                <span
                  aria-hidden="true"
                  className="flex size-10 shrink-0 items-center justify-center rounded-full bg-primary/10 text-sm font-semibold uppercase text-primary"
                >
                  {row.account.slice(0, 1)}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="break-all text-sm font-medium">{row.account}</p>
                  <p
                    role="status"
                    className="mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground"
                  >
                    {busy ? (
                      <LoaderCircleIcon
                        aria-hidden="true"
                        className="size-3 animate-spin motion-reduce:animate-none"
                      />
                    ) : (
                      <span
                        aria-hidden="true"
                        className={`size-2 rounded-full ${DOT[tone]}`}
                      />
                    )}
                    {status}
                  </p>
                  {row.message && (
                    <p className="mt-1 text-xs text-destructive">
                      {row.message}
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  {onReconnect && (
                    <button
                      type="button"
                      className={`inline-flex min-h-11 items-center gap-1.5 rounded-lg px-3 text-xs font-medium text-primary transition-colors hover:bg-primary/10 motion-reduce:transition-none ${focus}`}
                      title="Connect services or sign in again"
                      onClick={() => onReconnect(row.account)}
                    >
                      <PlugZapIcon aria-hidden="true" className="size-4" />
                      <span className="hidden sm:inline">Connect services</span>
                      <span className="sr-only sm:hidden">
                        Connect services / sign in again
                      </span>
                    </button>
                  )}
                  <button
                    type="button"
                    className={`flex min-h-11 min-w-11 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none ${focus}`}
                    aria-label={`Remove ${row.account}`}
                    title="Remove account"
                    disabled={!!pending}
                    onClick={() => setConfirm(row.account)}
                  >
                    <Trash2Icon aria-hidden="true" className="size-5" />
                  </button>
                </div>
              </div>
              <div className="mt-4 grid gap-2 sm:grid-cols-3">
                {SERVICES.map(({ cap, label, Icon }) => {
                  const on = row.capabilities.includes(cap);
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
                        className={`flex min-h-14 w-full items-center gap-3 rounded-xl border p-2.5 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none ${focus} ${on ? "border-primary/40 bg-primary/5 hover:bg-primary/10" : "bg-background hover:bg-accent"}`}
                      >
                        <IconBox on={on}>
                          <Icon aria-hidden={true} className="size-4" />
                        </IconBox>
                        <span className="min-w-0 flex-1">
                          <span className="block text-sm font-medium">
                            {label}
                          </span>
                          <span className="block text-xs text-muted-foreground">
                            {reason ? "Unavailable" : on ? "On" : "Off"}
                          </span>
                        </span>
                        <Switch on={on} />
                      </button>
                      {reason && (
                        <p className="mt-1 text-xs text-muted-foreground">
                          {reason}
                        </p>
                      )}
                    </div>
                  );
                })}
              </div>
              {teamsOn && onCalls && row.calls !== undefined && (
                <div
                  data-teams-options
                  className="mt-2 flex items-center gap-3 rounded-xl border border-dashed bg-muted/40 p-2.5"
                >
                  <IconBox on={row.calls}>
                    {row.calls ? (
                      <PhoneIcon aria-hidden="true" className="size-4" />
                    ) : (
                      <PhoneOffIcon aria-hidden="true" className="size-4" />
                    )}
                  </IconBox>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium">Teams calls</p>
                    <p className="text-xs text-muted-foreground">
                      {row.calls
                        ? "Rings and is answered in the app."
                        : "Messages only: no ringing. Notifications and AI (MCP) access to the chats keep working."}
                    </p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={row.calls}
                    aria-label={`Calls of ${row.account}`}
                    disabled={!!pending}
                    onClick={() => onCalls(row.account, !row.calls)}
                    className={`flex min-h-11 min-w-11 items-center justify-center rounded-lg transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none ${focus}`}
                  >
                    <Switch on={row.calls} />
                  </button>
                </div>
              )}
              {confirm === row.account && (
                <div
                  role="alertdialog"
                  aria-modal="false"
                  aria-label={"Remove " + row.account}
                  className="mt-3 rounded-xl border border-destructive/40 bg-destructive/5 p-4"
                >
                  <div className="flex items-start gap-3">
                    <TriangleAlertIcon
                      aria-hidden="true"
                      className="mt-0.5 size-5 shrink-0 text-destructive"
                    />
                    <p className="text-sm">
                      Remove {row.account}? Saved mail and calendar access,
                      matching Teams slots and their local data will be deleted
                      from both apps. Microsoft mailbox contents stay unchanged.
                      This cannot be undone; adding again requires sign-in.
                    </p>
                  </div>
                  <div className="mt-3 flex flex-wrap justify-end gap-2">
                    <button
                      type="button"
                      className={`min-h-11 rounded-lg border bg-background px-4 text-sm transition-colors hover:bg-accent motion-reduce:transition-none ${focus}`}
                      disabled={!!pending}
                      onClick={() => setConfirm("")}
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      className={`inline-flex min-h-11 items-center gap-2 rounded-lg bg-destructive px-4 text-sm font-medium text-white transition-colors hover:bg-destructive/90 disabled:opacity-50 motion-reduce:transition-none ${focus}`}
                      disabled={!!pending}
                      onClick={() => onRemove?.(row.account)}
                    >
                      <Trash2Icon aria-hidden="true" className="size-4" />
                      Confirm removal
                    </button>
                  </div>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
