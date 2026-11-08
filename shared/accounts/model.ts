import type { AccountRow, Capability } from "./AccountPanel";
export function buildAccountRows(
  accounts: {
    account: string;
    capabilities: Capability[];
    status: string;
    message?: string;
    state?: string;
  }[],
  teams: {
    email: string;
    slot: number;
    state: string;
  }[],
  reportValid = true,
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
    });
  }
  for (const t of reportValid ? teams : []) {
    const key = t.email.trim().toLowerCase() || `Teams account ${t.slot}`;
    const row = rows.get(key);
    if (row) {
      if (t.state !== "stopped")
        row.capabilities = [...new Set([...row.capabilities, "chat" as const])];
      if (row.status !== "Sign-in needed") row.status = `Teams: ${t.state}`;
      row.unavailable = undefined;
    } else
      rows.set(key, {
        account: key,
        capabilities: t.state === "stopped" ? [] : ["chat"],
        status: `Teams: ${t.state}`,
        unavailable: {
          mail: "Connect mailbox first",
          calendar: "Connect mailbox first",
        },
      });
  }
  return [...rows.values()].sort((a, b) => a.account.localeCompare(b.account));
}

export function validTeamsReport(at: number, now = Date.now() / 1000): boolean {
  return Number.isFinite(at) && at > 0 && now >= at && now - at <= 600;
}
