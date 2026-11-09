"use client";

import { useEffect, useState } from "react";
import { cn } from "cn";
import { PRESENCE_DOT } from "./Avatar";
import { accName, call, mediaUrl, type Account } from "@/lib/client";
import { isPresence, PRESENCE_TEXT } from "@/shared/presence";
import type { SignIn } from "@/shared/accounts/ConnectForm";
import { Logo } from "@/shared/accounts/Identity";
import { labelFor, signInOfSlot, type AccountLabels } from "@/shared/accounts/model";

export type SharedNames = { enabled: boolean; labels: AccountLabels | null; signins: SignIn[] };
const NONE: SharedNames = { enabled: false, labels: null, signins: [] };

// The names and logos the calendar gives the owner's accounts and the guided sign-ins running (GET /api/shared-accounts),
// read every minute: the account menu and the settings show each account as Microsoft accounts does. Off for anyone
// but the owner of the shared accounts.
export function useSharedNames(): SharedNames {
  const [names, setNames] = useState(NONE);
  useEffect(() => {
    let live = true;
    const read = () =>
      call<{ enabled?: boolean; labels?: AccountLabels | null; signins?: SignIn[] }>("/api/shared-accounts", undefined, 0).then(
        (r) => live && setNames({ enabled: !!r.enabled, labels: r.labels ?? null, signins: r.signins ?? [] }),
        () => undefined,
      );
    void read();
    const t = setInterval(() => void read(), 60_000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, []);
  return names;
}

export type Identity = { name: string; address: string; logo?: string; signin?: SignIn };

// Who an account is, the same as in Microsoft accounts: the company name and logo of its address, the address under
// it. A slot a guided sign-in runs in is the address being signed in. An address the calendar has no name for (an
// account on another computer) goes by its Teams organization; without the calendar's names (another user) the Teams
// picture shows.
export function identityOf(a: Account, names: SharedNames): Identity {
  const signin = !a.email && !a.relay ? signInOfSlot(a.slot, names.signins) : undefined;
  const address = (a.email || a.shared || signin?.account || "").trim().toLowerCase();
  const label = labelFor(address, names.labels);
  return {
    name: label.name ?? (signin ? address : a.tenant || accName(a)),
    address,
    logo: label.logo ?? (!names.labels && a.av ? mediaUrl(a.av, a.slot) : undefined),
    signin,
  };
}

// The picture of an account: the shared logo, with the presence dot of Teams on the account menu button
export function AccountLogo({ id, presence, faded, className }: { id: Identity; presence?: string; faded?: boolean; className?: string }) {
  const p = isPresence(presence) ? presence : null;
  return (
    <Logo name={id.name} src={id.logo} faded={faded} className={className}>
      {p && (
        <span
          data-presence={p}
          role="img"
          aria-label={PRESENCE_TEXT[p]}
          title={PRESENCE_TEXT[p]}
          className={cn("absolute -right-0.5 -bottom-0.5 grid size-3 place-items-center rounded-full ring-2 ring-background", PRESENCE_DOT[p])}
        >
          {p === "dnd" ? <span className="h-0.5 w-1.5 rounded-full bg-white" /> : null}
        </span>
      )}
    </Logo>
  );
}
