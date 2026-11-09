"use client";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AccountRow } from "@/shared/accounts/AccountPanel";
import {
  Management,
  type AccountTransport,
} from "@/shared/accounts/Management";
import type { VoceLogo } from "@/lib/oe/types";
import { AvatarLogo } from "./ControlloLogo";
import { buildAccountRows, validTeamsReport } from "@/shared/accounts/model";
import { api } from "@/lib/oe/api";
import { useOe } from "./OeProvider";
export function MicrosoftAccounts({
  onAdd,
  initialAccount,
}: { onAdd?: () => void; initialAccount?: string } = {}) {
  const oe = useOe();
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const transport = useMemo<AccountTransport>(() => {
    // the calendars reload in the background, once for a burst of changes: a switch never waits for it
    const refresh = () => {
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        void oe.aggiornaCalendari().then(() => oe.ricarica());
      }, 800);
    };
    return {
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
        refresh();
      },
      remove: async (account) => {
        const r = await api("manage_microsoft", account, "remove");
        if (!r.ok) throw Error(r.error);
        refresh();
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
      cancel: async (account, id) => {
        const r = await api("cancel_microsoft_signin", account, id);
        if (!r.ok) throw Error(r.error || "Sign-in not cancelled");
      },
      connected: refresh,
    };
  }, [oe, onAdd]);
  useEffect(() => () => clearTimeout(timer.current), []);
  // the logo of each company, changed by clicking it as on the calendars' page (ControlloLogo): same names, same files
  const [loghi, setLoghi] = useState<Record<string, VoceLogo>>({});
  const caricaLoghi = useCallback(async () => {
    try {
      setLoghi((await api("get_loghi")).loghi);
    } catch {
      // without them the dialog does not show the chosen address, and still works
    }
  }, []);
  useEffect(() => void caricaLoghi(), [caricaLoghi]);
  const renderLogo = useCallback(
    (row: AccountRow) => {
      const nome = row.name ?? row.account;
      const colore = oe.calendari.find((c) => (c.alias || c.account) === nome)?.color ?? "#64748b";
      return (
        <AvatarLogo
          nome={nome}
          colore={colore}
          lato={28}
          voce={loghi[nome]}
          onCambiato={async () => {
            await Promise.all([oe.ricaricaPers(), caricaLoghi()]);
          }}
        />
      );
    },
    [oe, loghi, caricaLoghi],
  );
  return <Management transport={transport} initialAccount={initialAccount} renderLogo={renderLogo} />;
}
