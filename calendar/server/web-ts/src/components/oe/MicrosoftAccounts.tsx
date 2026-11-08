"use client";
import { useMemo } from "react";
import {
  Management,
  type AccountTransport,
} from "@/shared/accounts/Management";
import { buildAccountRows, validTeamsReport } from "@/shared/accounts/model";
import { api } from "@/lib/oe/api";
import { useOe } from "./OeProvider";
export function MicrosoftAccounts({ onAdd }: { onAdd?: () => void } = {}) {
  const oe = useOe();
  const transport = useMemo<AccountTransport>(
    () => ({
      load: async () => {
        const r = await api("get_condivisione_microsoft");
        if (!r.ok) throw Error(r.error || "Accounts unavailable");
        return {
          rows: buildAccountRows(
            r.accounts.map((a) => ({
              account: a.account,
              capabilities: a.shared?.capabilities ?? ["mail", "calendar"],
              status: a.status,
              message: a.message,
              state: a.teams?.state,
            })),
            r.teamsOnly,
            validTeamsReport(r.teamsAt),
          ),
          commands: r.commands,
        };
      },
      toggle: async (account, cap, on) => {
        const latest = await api("get_condivisione_microsoft");
        if (!latest.ok) throw Error(latest.error);
        if (
          cap === "chat" &&
          latest.teamsOnly.some((t) => t.email.trim().toLowerCase() === account)
        ) {
          const r = await api("manage_microsoft", account, "chat", on);
          if (!r.ok) throw Error(r.error);
          return;
        }
        const a = latest.accounts.find(
          (a) => a.account.toLowerCase() === account,
        );
        if (!a) throw Error("Connect mailbox first");
        const before = a.shared?.capabilities ?? ["mail", "calendar"];
        const caps = on
          ? [...new Set([...before, cap])]
          : before.filter((c) => c !== cap);
        const r = await api("set_condivisione_microsoft", account, caps);
        if (!r.ok) throw Error(r.error);
        await oe.aggiornaCalendari();
        oe.ricarica();
      },
      remove: async (account) => {
        const r = await api("manage_microsoft", account, "remove");
        if (!r.ok) throw Error(r.error);
        await oe.aggiornaCalendari();
        oe.ricarica();
      },
      add: () => onAdd?.(),
    }),
    [oe, onAdd],
  );
  return <Management transport={transport} />;
}
