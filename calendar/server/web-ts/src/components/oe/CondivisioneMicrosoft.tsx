"use client";

import { useCallback, useEffect, useState } from "react";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import type { AccountCondiviso, CapacitaCondivisa, SoloTeams } from "@/lib/oe/types";
import { useOe } from "./OeProvider";

const VOCI: { cap: CapacitaCondivisa; nome: string }[] = [
  { cap: "mail", nome: "Posta" },
  { cap: "calendar", nome: "Calendario" },
  { cap: "chat", nome: "Teams" },
];

/** Una casella che non ha mai scelto vale posta e calendario, come prima degli account condivisi. */
const capacitaDi = (a: AccountCondiviso | undefined): CapacitaCondivisa[] => (a?.shared ? a.shared.capabilities : ["mail", "calendar"]);

const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** Lo stato dell'account Teams come lo manda TeamsRelay, in parole. */
export function statoTeams(t: { slot: number; state: string } | null): string {
  if (!t) return "";
  const parole: Record<string, string> = { ok: "attivo", running: "attivo", stopped: "spento", starting: "in avvio", login: "accesso da rifare", "signed-out": "accesso da rifare" };
  return `account Teams ${t.slot}, ${parole[t.state] ?? t.state}`;
}

/**
 * Posta, calendario e Teams di una casella Microsoft, ognuno acceso o spento con un clic: lo stesso elenco che TeamsRelay mostra nelle
 * sue Impostazioni, e un cambio da una parte si vede dall'altra. Un solo accesso, quello di questa casella: Teams acceso fa girare su
 * di lui un account Teams senza browser; spento lo ferma e ne tiene le chat.
 */
export function CondivisioneMicrosoft({ account }: { account: string }) {
  const oe = useOe();
  const [voce, setVoce] = useState<AccountCondiviso | null | undefined>(undefined);
  const [teamsAt, setTeamsAt] = useState(0);
  const [lavoro, setLavoro] = useState<CapacitaCondivisa | "">("");
  const [errore, setErrore] = useState("");

  const leggi = useCallback(async () => {
    try {
      const r = await api("get_condivisione_microsoft");
      if (!r.ok) throw new Error(r.error ?? "Stato degli account non letto");
      setVoce(r.accounts.find((a) => a.account.toLowerCase() === account.toLowerCase()) ?? null);
      setTeamsAt(r.teamsAt ?? 0);
      setErrore("");
    } catch (e) {
      setVoce(null);
      setErrore(messaggio(e));
    }
  }, [account]);

  useEffect(() => {
    void leggi();
  }, [leggi]);

  // Un servizio alla volta, sull'elenco com'è adesso e non come lo si è letto all'apertura: un cambio fatto intanto dalle
  // Impostazioni di TeamsRelay resta.
  const cambia = async (cap: CapacitaCondivisa, acceso: boolean) => {
    setLavoro(cap);
    setErrore("");
    try {
      const ora = await api("get_condivisione_microsoft");
      if (!ora.ok) throw new Error(ora.error ?? "Stato degli account non letto");
      const prima = capacitaDi(ora.accounts.find((a) => a.account.toLowerCase() === account.toLowerCase()));
      const capacita = acceso ? [...new Set([...prima, cap])] : prima.filter((c) => c !== cap);
      const r = await api("set_condivisione_microsoft", account, capacita);
      if (!r.ok) throw new Error(r.error ?? "Non cambiato");
      await leggi();
      if (cap === "calendar") {
        await oe.aggiornaCalendari();
        oe.ricarica();
      }
    } catch (e) {
      setErrore(messaggio(e));
    } finally {
      setLavoro("");
    }
  };

  if (voce === undefined) return <Spinner className="size-4" />;
  const capacita = capacitaDi(voce ?? undefined);
  const teams = voce?.teams ?? null;

  return (
    <div data-condivisione={account} className="flex flex-col gap-2 text-[13px]">
      <div className="flex flex-wrap items-center gap-4">
        {VOCI.map((v) => (
          <label key={v.cap} className="flex items-center gap-2">
            <Switch
              data-capacita={v.cap}
              checked={capacita.includes(v.cap)}
              disabled={!!lavoro || !voce}
              onCheckedChange={(si) => void cambia(v.cap, si)}
              aria-label={`${v.nome} di ${account}`}
            />
            {v.nome}
            {lavoro === v.cap && <Spinner className="size-3.5" />}
          </label>
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        {capacita.includes("chat")
          ? teams
            ? `Teams: ${statoTeams(teams)}, senza browser, con questo accesso.`
            : teamsAt
              ? "Teams: in avvio con questo accesso."
              : "Teams: acceso, TeamsRelay non ha ancora mandato il suo stato."
          : "Un solo accesso, questo, per posta, calendario e Teams: lo stesso elenco c'è nelle Impostazioni di TeamsRelay."}
      </p>
      {errore && (
        <p role="alert" className="text-xs text-destructive">
          {errore}
        </p>
      )}
    </div>
  );
}

/** Gli account Teams con il solo accesso del loro browser in TeamsRelay: qui non hanno posta né calendario. */
export function PannelloSoloTeams({ voci }: { voci: SoloTeams[] }) {
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-lg font-semibold">Solo in Teams</h2>
      <p className="text-muted-foreground">
        Account Teams con l&apos;accesso fatto nel loro browser in TeamsRelay. Per avere anche posta e calendario, aggiungi qui la stessa casella e accendi Teams: l&apos;account
        Teams passa a questo accesso, senza browser.
      </p>
      <ul id="solo-teams" className="flex flex-col divide-y">
        {voci.map((t) => (
          <li key={t.slot} className="py-2">
            <span className="font-medium">{t.email || `Account ${t.slot}`}</span> <span className="text-muted-foreground">{statoTeams(t)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
