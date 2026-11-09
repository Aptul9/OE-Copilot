import type { AccountRow, Capability } from "./AccountPanel";
// The name and logo of each address as the calendar sidebar shows them, and the companies whose meetings are forwarded into
// another mailbox (company name to that mailbox's name). Logos are URLs of the calendar, or data URLs in TeamsRelay.
export type AccountLabels = {
  names?: Record<string, string>;
  logos?: Record<string, string>;
  forwarded?: Record<string, string>;
};
function labelled(rows: AccountRow[], labels?: AccountLabels): AccountRow[] {
  for (const row of rows) {
    const name = labels?.names?.[row.account];
    if (!name) continue;
    row.name = name;
    row.logo = labels?.logos?.[name];
    const via = labels?.forwarded?.[name];
    if (via && !row.capabilities.includes("calendar")) row.via = via;
  }
  return rows.sort(
    (a, b) =>
      (a.name ?? a.account).localeCompare(b.name ?? b.account, undefined, {
        sensitivity: "base",
      }) || a.account.localeCompare(b.account),
  );
}
export function buildAccountRows(
  accounts: {
    account: string;
    capabilities: Capability[];
    status: string;
    message?: string;
    state?: string;
    calls?: boolean | null;
  }[],
  teams: {
    email: string;
    slot: number;
    state: string;
    calls?: boolean | null;
  }[],
  reportValid = true,
  labels?: AccountLabels,
): AccountRow[] {
  const rows = new Map<string, AccountRow>();
  for (const a of accounts) {
    const key = a.account.trim().toLowerCase();
    if (!key) continue;
    const previous = rows.get(key);
    rows.set(key, {
      account: key,
      capabilities: [
        ...new Set([...(previous?.capabilities ?? []), ...a.capabilities]),
      ],
      status: !reportValid
        ? "Teams: unknown (report unavailable)"
        : a.status === "error" || a.status === "errore"
          ? "Sign-in needed"
          : a.state
            ? `Teams: ${a.state}`
            : "Connected",
      message: a.message,
      unavailable:
        !reportValid && !a.capabilities.includes("chat")
          ? { chat: "Teams report unavailable. Cannot enable safely." }
          : undefined,
      calls: reportValid && typeof a.calls === "boolean" ? a.calls : undefined,
    });
  }
  for (const t of reportValid ? teams : []) {
    const key = t.email.trim().toLowerCase() || `Teams account ${t.slot}`;
    const row = rows.get(key);
    const calls = typeof t.calls === "boolean" ? t.calls : undefined;
    if (row) {
      if (t.state !== "stopped")
        row.capabilities = [...new Set([...row.capabilities, "chat" as const])];
      if (row.status !== "Sign-in needed") row.status = `Teams: ${t.state}`;
      row.unavailable = undefined;
      if (calls !== undefined) row.calls = calls;
    } else
      rows.set(key, {
        account: key,
        capabilities: t.state === "stopped" ? [] : ["chat"],
        status: `Teams: ${t.state}`,
        unavailable: {
          mail: "Connect mailbox first",
          calendar: "Connect mailbox first",
        },
        calls,
      });
  }
  return labelled([...rows.values()], labels);
}

export function validTeamsReport(at: number, now = Date.now() / 1000): boolean {
  return Number.isFinite(at) && at > 0 && now >= at && now - at <= 600;
}
