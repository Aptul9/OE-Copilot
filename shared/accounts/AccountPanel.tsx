"use client";
import { useState } from "react";
import type { ComponentType, ReactNode } from "react";
import {
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
import { Logo } from "./Identity";
export type Capability = "mail" | "calendar" | "chat";
export type AccountRow = {
  account: string;
  capabilities: Capability[];
  status: string;
  message?: string;
  unavailable?: Partial<Record<Capability, string>>;
  // Teams calls of an account without a browser; undefined when calls cannot be switched for it
  calls?: boolean;
  // the name and logo the calendar sidebar shows for this address
  name?: string;
  logo?: string;
  // the mailbox the company's meetings are forwarded into, when the account has no calendar of its own
  via?: string;
  // the TeamsRelay slot of a Teams account not signed in yet (no address): a guided sign-in may be running in it
  slot?: number;
  // an account being signed in for the first time: nothing to switch or remove yet, its sign-in has a Cancel
  pending?: boolean;
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
export const focus =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
// 32 px with a mouse, 44 px on touch screens
export const ICON_BUTTON = `inline-flex size-8 shrink-0 items-center justify-center rounded-lg transition-colors pointer-coarse:size-11 disabled:cursor-not-allowed motion-reduce:transition-none ${focus}`;
function ServiceToggle({
  label,
  name,
  Icon,
  on,
  hint,
  dashed,
  disabled,
  saving,
  onClick,
}: {
  label: string;
  name: string;
  Icon: ComponentType<{ className?: string; "aria-hidden"?: boolean }>;
  on: boolean;
  hint: string;
  dashed?: boolean;
  disabled?: boolean;
  // the change is on its way: the switch already shows the state asked, with a spinner, and takes no second click
  saving?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-busy={saving || undefined}
      aria-label={`${label} of ${name}`}
      title={saving ? `${label}: saving...` : hint}
      disabled={disabled || saving}
      onClick={onClick}
      className={`${ICON_BUTTON} ${
        on
          ? "bg-primary text-primary-foreground hover:bg-primary/90"
          : dashed
            ? "border border-dashed border-primary/60 text-primary"
            : "border text-muted-foreground hover:bg-accent hover:text-foreground"
      } ${disabled && !dashed && !saving ? "opacity-40" : ""}`}
    >
      {saving ? (
        <LoaderCircleIcon aria-hidden="true" className="size-4 animate-spin motion-reduce:animate-none" />
      ) : (
        <Icon aria-hidden={true} className="size-4" />
      )}
    </button>
  );
}
export function AccountPanel({
  rows,
  busy = [],
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
  expanded,
  inline,
  renderLogo,
}: {
  rows: AccountRow[];
  // changes on their way: "account|mail", "account|calendar", "account|chat", "account|calls", or "account|*" for the whole row
  busy?: string[];
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
  // the account whose row shows `inline` under it (its connection form and progress)
  expanded?: string;
  inline?: ReactNode;
  // the app's own logo control, where the logo can be changed (the calendar's); without it the logo only shows
  renderLogo?: (row: AccountRow) => ReactNode;
}) {
  const [confirm, setConfirm] = useState("");
  return (
    <section
      className="overflow-hidden rounded-xl border bg-card text-card-foreground"
      data-account-panel
      aria-label="Microsoft accounts"
    >
      <header className="flex items-center justify-between gap-3 border-b px-3 py-2">
        <h2 className="text-sm font-semibold">Microsoft accounts</h2>
        <button
          type="button"
          className={`inline-flex h-8 items-center gap-1.5 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50 pointer-coarse:h-11 motion-reduce:transition-none ${focus}`}
          disabled={loading}
          onClick={onAdd}
        >
          <PlusIcon aria-hidden="true" className="size-3.5" />
          Add account
        </button>
      </header>
      {children}
      {error && (
        <p
          role="alert"
          className="flex items-start gap-2 border-b bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          <TriangleAlertIcon aria-hidden="true" className="size-3.5 shrink-0" />
          {error}
        </p>
      )}
      {loading && (
        <p
          role="status"
          className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground"
        >
          <LoaderCircleIcon
            aria-hidden="true"
            className="size-3.5 animate-spin motion-reduce:animate-none"
          />
          Loading accounts...
        </p>
      )}
      {feedback && (
        <p
          role="status"
          className="flex items-center gap-2 border-b bg-muted/40 px-3 py-2 text-xs"
        >
          <CheckIcon aria-hidden="true" className="size-3.5 shrink-0 text-success" />
          {feedback}
        </p>
      )}
      {!loading && !rows.length && (
        <p className="px-3 py-6 text-center text-sm text-muted-foreground">
          No accounts connected.
        </p>
      )}
      <ul className="divide-y">
        {rows.map((row) => {
          const name = row.name ?? row.account;
          const locked = busy.includes(`${row.account}|*`);
          const saving = (what: string) =>
            locked || busy.includes(`${row.account}|${what}`);
          const changing = locked || busy.some((k) => k.startsWith(`${row.account}|`));
          const rowBusy = locked || busyAccounts.includes(row.account);
          const status = locked
            ? "Saving..."
            : busyAccounts.includes(row.account)
              ? "Connecting services..."
              : row.status;
          const tone = rowBusy ? "info" : toneOf(row.status, row.message);
          const teamsOn = row.capabilities.includes("chat");
          const showCalls = teamsOn && !!onCalls && row.calls !== undefined;
          return (
            <li
              key={row.account}
              data-account={row.account}
              aria-busy={changing}
              className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2"
            >
              {renderLogo && !row.pending ? renderLogo(row) : <Logo name={name} src={row.logo} />}
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-1.5 text-sm font-medium">
                  <span className="truncate">{name}</span>
                  {rowBusy ? (
                    <LoaderCircleIcon
                      aria-hidden="true"
                      className="size-3 shrink-0 animate-spin text-primary motion-reduce:animate-none"
                    />
                  ) : (
                    <span
                      aria-hidden="true"
                      title={status}
                      className={`size-1.5 shrink-0 rounded-full ${DOT[tone]}`}
                    />
                  )}
                </p>
                <p
                  role="status"
                  className="truncate text-xs text-muted-foreground"
                  title={[row.account, status, row.message].filter(Boolean).join(" · ")}
                >
                  {row.name ? row.account : ""}
                  {tone === "ok" ? (
                    <span className="sr-only"> {status}</span>
                  ) : (
                    <span className={tone === "warn" ? "text-destructive" : ""}>
                      {row.name ? " · " : ""}
                      {status}
                    </span>
                  )}
                </p>
              </div>
              <div
                role="group"
                aria-label={`Services of ${name}`}
                className="order-last flex basis-full items-center gap-1 pl-10 sm:order-none sm:basis-auto sm:pl-0"
              >
                {SERVICES.map(({ cap, label, Icon }) => {
                  const on = row.capabilities.includes(cap);
                  const via = cap === "calendar" && !on ? row.via : undefined;
                  const reason = row.unavailable?.[cap];
                  return (
                    <ServiceToggle
                      key={cap}
                      label={label}
                      name={name}
                      Icon={Icon}
                      on={on}
                      dashed={!!via}
                      hint={
                        via
                          ? `${label}: arrives forwarded via ${via}. Connect services to read it directly.`
                          : reason
                            ? `${label}: ${reason}`
                            : `${label}: ${on ? "on" : "off"}`
                      }
                      saving={saving(cap)}
                      disabled={
                        !!reason || (busyAccounts.includes(row.account) && !on)
                      }
                      onClick={() => onToggle(row.account, cap, !on)}
                    />
                  );
                })}
                {showCalls ? (
                  <ServiceToggle
                    label="Calls"
                    name={name}
                    Icon={row.calls ? PhoneIcon : PhoneOffIcon}
                    on={!!row.calls}
                    hint={
                      row.calls
                        ? "Teams calls: on. Rings and is answered in the app."
                        : "Teams calls: off. Messages only: no ringing. Notifications and AI (MCP) access to the chats keep working."
                    }
                    saving={saving("calls")}
                    onClick={() => onCalls(row.account, !row.calls)}
                  />
                ) : (
                  <span aria-hidden="true" className="size-8 shrink-0 pointer-coarse:size-11" />
                )}
              </div>
              <div className="flex items-center">
                {onReconnect && (
                  <button
                    type="button"
                    className={`${ICON_BUTTON} ${tone === "warn" ? "text-destructive hover:bg-destructive/10" : "text-muted-foreground hover:bg-accent hover:text-foreground"}`}
                    aria-label={`Connect services of ${name}`}
                    aria-expanded={expanded === row.account}
                    title="Connect services or sign in again"
                    onClick={() => onReconnect(row.account)}
                  >
                    <PlugZapIcon aria-hidden="true" className="size-4" />
                  </button>
                )}
                {!row.pending ? (
                  <button
                    type="button"
                    className={`${ICON_BUTTON} text-muted-foreground hover:bg-destructive/10 hover:text-destructive disabled:opacity-50`}
                    aria-label={`Remove ${row.account}`}
                    title="Remove account"
                    disabled={locked}
                    onClick={() => setConfirm(row.account)}
                  >
                    <Trash2Icon aria-hidden="true" className="size-4" />
                  </button>
                ) : (
                  <span aria-hidden="true" className="size-8 shrink-0 pointer-coarse:size-11" />
                )}
              </div>
              {expanded === row.account && inline && (
                <div className="order-last basis-full">{inline}</div>
              )}
              {confirm === row.account && (
                <div
                  role="alertdialog"
                  aria-modal="false"
                  aria-label={"Remove " + row.account}
                  className="order-last flex basis-full flex-wrap items-center gap-2 rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs"
                >
                  <TriangleAlertIcon
                    aria-hidden="true"
                    className="size-4 shrink-0 text-destructive"
                  />
                  <p className="min-w-0 flex-1">
                    Remove {row.account}? Saved mail and calendar access,
                    matching Teams slots and their local data will be deleted
                    from both apps. Microsoft mailbox contents stay unchanged.
                    This cannot be undone; adding again requires sign-in.
                  </p>
                  <button
                    type="button"
                    className={`h-8 rounded-lg border bg-background px-3 hover:bg-accent pointer-coarse:h-11 ${focus}`}
                    disabled={locked}
                    onClick={() => setConfirm("")}
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    className={`inline-flex h-8 items-center gap-1.5 rounded-lg bg-destructive px-3 font-medium text-white hover:bg-destructive/90 disabled:opacity-50 pointer-coarse:h-11 ${focus}`}
                    disabled={locked}
                    onClick={() => onRemove?.(row.account)}
                  >
                    <Trash2Icon aria-hidden="true" className="size-3.5" />
                    Confirm removal
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
