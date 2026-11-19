// The desktop tab with its switcher of the accounts (/remote) on a page of its own, for test/desktop-switcher.test.ts:
// the accounts of one user, the one asked for in ?account=N.
import { createRoot, hydrateRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { DesktopSwitcher } from "@/components/DesktopSwitcher";
import { RemoteExported } from "@/components/ExportedPages";
import type { Account } from "@/lib/client";

const base: Account = {
  slot: 0,
  name: "",
  email: "",
  tenant: "",
  av: "",
  presence: "",
  teams: "ok",
  overall: "green",
  stopped: false,
  unread: 0,
  unreadActivity: [],
  missedCalls: [],
  activityIds: [],
  added: 1790000000,
  desktop: "",
  checkEvery: 0,
  checked: 0,
  checkResult: "",
  nextCheck: 0,
  checking: false,
  relay: false,
  host: "",
  relaySeen: 0,
  callAudio: false,
  light: false,
  refreshing: false,
};

const q = new URLSearchParams(location.search);
// ?counts: something waits in some accounts (the test stores what this device has seen of them)
const counts = q.has("counts");

const accounts: Account[] = [
  {
    ...base,
    slot: 1,
    name: "Test User",
    email: "test.user@contoso.example",
    tenant: "Contoso Cruises",
    desktop: "/api/desktop/1",
    ...(counts && { unread: 2, unreadActivity: ["n2", "n1"], activityIds: ["n2", "n1"] }),
  },
  {
    ...base,
    slot: 2,
    name: "User Test",
    email: "user.test@fabrikam.example",
    tenant: "",
    desktop: "/api/desktop/2",
    ...(counts && { missedCalls: ["c2", "c1"], activityIds: ["c2", "c1"] }),
  },
  { ...base, slot: 3, name: "Relay User", tenant: "Relay Srl", relay: true, ...(counts && { unread: 1 }) },
  { ...base, slot: 4, name: "Stopped User", tenant: "Stopped Srl", stopped: true, desktop: "/api/desktop/4", ...(counts && { unread: 4 }) },
  { ...base, slot: 5, name: "Checked User", tenant: "Checked Srl", checkEvery: 3600, desktop: "/api/desktop/5" },
];
// for the test, which answers /api/accounts with them changed
(window as unknown as { fixture: Account[] }).fixture = accounts;

const initial = Number(q.get("account"));
const root = document.getElementById("root")!;
if (q.has("exported")) {
  // ?exported: the page of the Rust web app, the HTML of the build (no account) hydrated with the values of the server
  root.innerHTML = renderToString(<RemoteExported />);
  (window as unknown as { __TR__: unknown }).__TR__ = { accounts, initial };
  hydrateRoot(root, <RemoteExported />);
} else {
  createRoot(root).render(<DesktopSwitcher accounts={accounts} initial={initial} />);
}
