"use client";
import { useMemo } from "react";
import {
  Management,
  type AccountTransport,
} from "@/shared/accounts/Management";
import { buildAccountRows, validTeamsReport } from "@/shared/accounts/model";
import { api } from "@/lib/oe/api";
import { useOe } from "./OeProvider";
export function MicrosoftAccounts({
  onAdd,
  initialAccount,
}: { onAdd?: () => void; initialAccount?: string } = {}) {
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
              calls: a.teams?.calls,
            })),
            r.teamsOnly,
            validTeamsReport(r.teamsAt),
            r.labels,
          ),
          commands: r.commands,
          signins: r.signins,
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
      setCalls: async (account, on) => {
        const r = await api("manage_microsoft", account, "calls", on);
        if (!r.ok) throw Error(r.error);
      },
      add: () => onAdd?.(),
      connect: async (account, capabilities) => {
        const r = await api("connect_microsoft", account, capabilities);
        if (!r.ok) throw Error(r.error || "Sign-in failed");
      },
      connected: () => {
        void oe.aggiornaCalendari();
        oe.ricarica();
      },
    }),
    [oe, onAdd],
  );
  return <Management transport={transport} initialAccount={initialAccount} />;
}
