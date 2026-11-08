"use client";

import { ExternalLinkIcon, UsersRoundIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { call, errorText, post } from "@/lib/client";

type Capability = "mail" | "calendar" | "chat";
type SharedRow = { account: string; capabilities: Capability[]; shared: boolean; status: "ok" | "error"; message: string; slot: number; running: boolean; teams: string };
type TeamsOnly = { slot: number; email: string; name: string; running: boolean; teams: string };
type SharedList = { enabled: boolean; addUrl: string; accounts: SharedRow[]; teamsOnly: TeamsOnly[]; error?: string };

const SWITCHES: { cap: Capability; label: string }[] = [
  { cap: "mail", label: "Mail" },
  { cap: "calendar", label: "Calendar" },
  { cap: "chat", label: "Teams" },
];

// What the Teams account of a row is doing, in words
function teamsState(running: boolean, teams: string, slot: number): string {
  if (!slot) return "";
  if (!running) return "stopped";
  if (teams === "ok") return "running";
  if (teams === "starting" || !teams) return "starting";
  if (teams === "login" || teams === "signed-out") return "sign-in needed in the calendar";
  return teams;
}

// null when the list could not be read: the list shown stays
const fetchList = () =>
  call<SharedList>("/api/shared-accounts", undefined, 0).then(
    (d) => d,
    () => null,
  );

// The one list of Microsoft accounts, signed in once in the calendar: mail, calendar and Teams of each switched on or off
// here or in the Account window of the calendar, the same list on both sides. Shown only to the owner of the shared
// accounts, and only when the browsers container reaches the calendar.
export function SharedAccounts({ onChange }: { onChange?: () => void }) {
  const [list, setList] = useState<SharedList | null>(null);
  const [saving, setSaving] = useState("");

  useEffect(() => {
    const show = (l: SharedList | null) => setList((shown) => l ?? shown);
    void fetchList().then(show);
    const timer = setInterval(() => void fetchList().then(show), 15_000);
    return () => clearInterval(timer);
  }, []);

  async function toggle(row: SharedRow, cap: Capability, on: boolean) {
    const capabilities = on ? [...new Set([...row.capabilities, cap])] : row.capabilities.filter((c) => c !== cap);
    setSaving(row.account);
    try {
      const r = await post<SharedList>("/api/shared-accounts", { account: row.account, capabilities }, 0);
      setList(r);
      onChange?.();
      const name = SWITCHES.find((s) => s.cap === cap)!.label;
      if (r.error) toast.error(`${row.account}: ${name} ${on ? "on" : "off"}, not applied yet`, { description: r.error });
      else toast.success(`${row.account}: ${name} ${on ? "on" : "off"}`, { description: "The calendar shows the same change." });
    } catch (e) {
      toast.error(errorText(e, "Not changed"));
    } finally {
      setSaving("");
    }
  }

  if (list === null) return null;
  if (!list.enabled) return null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <UsersRoundIcon className="size-4 text-muted-foreground" />
          Microsoft accounts
        </CardTitle>
        <CardDescription>
          Signed in once, in the calendar. Mail, calendar and Teams of each account go on and off here or in the Account window of the calendar: both show the
          same list. Teams on runs the account here without a browser; off stops it and keeps its chats.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {!list.accounts.length ? (
          <p className="text-sm text-muted-foreground">No Microsoft account in the calendar yet.</p>
        ) : (
          <div className="divide-y">
            {list.accounts.map((a) => {
              const state = teamsState(a.running, a.teams, a.slot);
              return (
                <div key={a.account} data-shared-account={a.account} className="flex flex-wrap items-center gap-3 py-3 first:pt-0 last:pb-0">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{a.account}</p>
                    <p className="text-xs text-muted-foreground">
                      {a.status === "error" ? `Sign in again in the calendar: ${a.message}` : state ? `Teams ${state}` : a.capabilities.includes("chat") ? "Teams starting" : "Not in Teams"}
                    </p>
                  </div>
                  {a.status === "error" && <Badge variant="destructive">Sign-in needed</Badge>}
                  <div className="flex flex-wrap items-center gap-4">
                    {SWITCHES.map((s) => (
                      <label key={s.cap} className="flex items-center gap-2 text-sm">
                        <Switch
                          checked={a.capabilities.includes(s.cap)}
                          onCheckedChange={(on) => void toggle(a, s.cap, on)}
                          disabled={saving === a.account}
                          aria-label={`${s.label} of ${a.account}`}
                          data-capability={s.cap}
                        />
                        {s.label}
                      </label>
                    ))}
                    {saving === a.account && <Spinner className="size-4" />}
                  </div>
                </div>
              );
            })}
          </div>
        )}
        {list.teamsOnly.length > 0 && (
          <div className="flex flex-col gap-1">
            <p className="text-sm font-medium">Teams only, signed in here</p>
            {list.teamsOnly.map((t) => (
              <p key={t.slot} className="text-sm text-muted-foreground">
                {t.email || t.name || `Account ${t.slot}`}: {teamsState(t.running, t.teams, t.slot)}. Signed in once in the calendar with Teams on, it shows above with its
                mail and calendar, and this Teams account runs on that sign-in.
              </p>
            ))}
          </div>
        )}
        {list.addUrl && (
          <div>
            <Button asChild variant="outline" className="h-10 md:h-9">
              <a href={list.addUrl} target="_blank" rel="noreferrer">
                <ExternalLinkIcon />
                Add a Microsoft account in the calendar
              </a>
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
