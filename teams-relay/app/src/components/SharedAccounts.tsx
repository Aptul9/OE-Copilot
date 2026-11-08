"use client";
import { useMemo } from "react";
import {
  Management,
  type AccountTransport,
} from "@/shared/accounts/Management";
import { buildAccountRows } from "@/shared/accounts/model";
import { call, post } from "@/lib/client";
type List = {
  enabled: boolean;
  addUrl: string;
  accounts: {
    account: string;
    capabilities: ("mail" | "calendar" | "chat")[];
    status: string;
    message: string;
    slot: number;
    running: boolean;
    teams: string;
  }[];
  teamsOnly: { email: string; slot: number; running: boolean; teams: string }[];
  commands?: { account: string; error?: string }[];
  error?: string;
};
export function SharedAccounts({ onChange }: { onChange?: () => void }) {
  const transport = useMemo<AccountTransport>(() => {
    let addUrl = "";
    return {
      load: async () => {
        const l = await call<List>("/api/shared-accounts", undefined, 0);
        if (!l.enabled) throw Error("Shared accounts unavailable");
        addUrl = l.addUrl;
        return {
          rows: buildAccountRows(
            l.accounts.map((a) => ({
              ...a,
              state: a.slot ? (a.running ? a.teams : "stopped") : "",
            })),
            l.teamsOnly.map((t) => ({
              ...t,
              state: t.running ? t.teams : "stopped",
            })),
          ),
          commands: l.commands,
        };
      },
      toggle: async (account, capability, on) => {
        const r = await post<List>(
          "/api/shared-accounts",
          { account, capability, on },
          0,
        );
        if (r.error) throw Error(r.error);
        onChange?.();
      },
      remove: async (account) => {
        await post("/api/shared-accounts", { account, operation: "remove" }, 0);
        onChange?.();
      },
      add: () => {
        if (!addUrl) throw Error("Calendar sign-in URL unavailable");
        const url = new URL(addUrl, window.location.href);
        url.searchParams.set("add-account", "1");
        window.location.assign(url.href);
      },
    };
  }, [onChange]);
  return <Management transport={transport} />;
}
