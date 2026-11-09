"use client";
import { useMemo, useRef } from "react";
import { CameraIcon } from "lucide-react";
import { Logo } from "@/shared/accounts/Identity";
import {
  Management,
  type AccountTransport,
} from "@/shared/accounts/Management";
import { buildAccountRows, type AccountLabels } from "@/shared/accounts/model";
import { call, post } from "@/lib/client";
import type { SignIn } from "@/shared/accounts/ConnectForm";
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
    calls: boolean | null;
  }[];
  teamsOnly: {
    email: string;
    slot: number;
    running: boolean;
    teams: string;
    calls: boolean | null;
  }[];
  commands?: { account: string; error?: string }[];
  signins?: SignIn[];
  labels?: AccountLabels | null;
  error?: string;
};
// adding: opened from "Add a Teams account" of the account menu, with the form to add one already open
export function SharedAccounts({ onChange, adding }: { onChange?: () => void; adding?: boolean }) {
  const addUrl = useRef("");
  const transport = useMemo<AccountTransport>(() => {
    return {
      load: async () => {
        const l = await call<List>("/api/shared-accounts", undefined, 0);
        if (!l.enabled) throw Error("Shared accounts unavailable");
        addUrl.current = l.addUrl;
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
            true,
            l.labels ?? undefined,
          ),
          commands: l.commands,
          signins: l.signins,
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
      setCalls: async (account, on) => {
        await post(
          "/api/shared-accounts",
          { account, operation: "calls", on },
          0,
        );
        onChange?.();
      },
      connect: async (account, capabilities) => {
        await post(
          "/api/shared-accounts",
          { operation: "connect", account, capabilities },
          0,
        );
      },
      cancel: async (account, id) => {
        await post(
          "/api/shared-accounts",
          { operation: "signin-cancel", account, id },
          0,
        );
        onChange?.();
      },
      connected: () => onChange?.(),
      add: () => {
        if (!addUrl.current) throw Error("Calendar sign-in URL unavailable");
        const url = new URL(addUrl.current, window.location.href);
        url.searchParams.set("add-account", "1");
        window.location.assign(url.href);
      },
    };
  }, [onChange]);
  return <Management transport={transport} initialAccount={adding ? "" : undefined} renderLogo={(row) => { const url = new URL(addUrl.current, window.location.href); url.searchParams.set("account-logos", "1"); return <a href={url.href} target="_blank" rel="noreferrer" title={`Change logo of ${row.name ?? row.account} in calendar`} aria-label={`Change logo of ${row.name ?? row.account} in calendar`} className="group relative shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-ring"><Logo name={row.name ?? row.account} src={row.logo} /><span aria-hidden="true" className="absolute inset-0 grid place-items-center rounded-full bg-black/50 text-white opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"><CameraIcon className="size-3.5" /></span></a>; }} />;
}
