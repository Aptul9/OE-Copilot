"use client";

import { useSyncExternalStore } from "react";
import { AdminPanel } from "@/components/AdminPanel";
import { App } from "@/components/App";
import { DesktopSwitcher } from "@/components/DesktopSwitcher";
import { LoginForm } from "@/components/LoginForm";
import { Settings } from "@/components/Settings";
import { StatusDashboard } from "@/components/StatusDashboard";
import type { Account } from "@/lib/client";
import type { StatusAnswer } from "@/lib/status-pages";

// The pages of the exported UI (TR_EXPORT=1, app/**/*.export.tsx) for the Rust web app (webapp-rs): the session check is
// the server's, and the values the page got from its server component arrive in window.__TR__, written in the head
// before any script.

const noChange = () => () => {};

/**
 * The values of the server. The render at build time (and the first render in the browser) uses the defaults, so the
 * HTML has the page and not a blank sheet until the JavaScript arrives.
 */
function useServerValues<T>(defaults: T): T {
  return useSyncExternalStore(
    noChange,
    () => (window as { __TR__?: T }).__TR__ ?? defaults,
    () => defaults,
  );
}

type User = { name: string; email: string; role: string };
const NO_USER: User = { name: "", email: "", role: "user" };

export function HomeExported() {
  const v = useServerValues({ user: NO_USER, desktopUrl: "" });
  return <App user={v.user} desktopUrl={v.desktopUrl} />;
}

export function AdminExported() {
  const v = useServerValues({ selfId: "" });
  return <AdminPanel selfId={v.selfId} />;
}

export function LoginExported() {
  const v = useServerValues({ next: "/" });
  return <LoginForm next={v.next} />;
}

export function SettingsExported() {
  const v = useServerValues({ user: NO_USER, passwordManaged: false });
  return <Settings user={v.user} passwordManaged={v.passwordManaged} />;
}

const NO_REMOTE = { accounts: [] as Account[], initial: 0 };

// The switcher keeps the address of the desktop from its first render: the values of the server mount it again, or
// the first render in the browser (no account) leaves the desktop with no address, a blank page
export function RemoteExported() {
  const v = useServerValues(NO_REMOTE);
  return <DesktopSwitcher key={v === NO_REMOTE ? "build" : "server"} accounts={v.accounts} initial={v.initial} />;
}

// One page for every status slug: the server sends status.html with the answer of the slug
export function StatusExported() {
  const v = useServerValues<{ slug: string; initial: StatusAnswer | null; serverNow: number }>({ slug: "", initial: null, serverNow: 0 });
  return v.initial ? <StatusDashboard key={v.slug} slug={v.slug} initial={v.initial} serverNow={v.serverNow} /> : null;
}
