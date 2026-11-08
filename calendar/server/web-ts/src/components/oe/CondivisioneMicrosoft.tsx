"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { api } from "@/lib/oe/api";
import type { CapacitaCondivisa, Condivisione } from "@/lib/oe/types";

const VOCI: { cap: CapacitaCondivisa; nome: string; nota: string }[] = [
  { cap: "mail", nome: "Posta", nota: "il gateway posta legge questa casella" },
  { cap: "calendar", nome: "Calendario", nota: "senza, il calendario di questa casella non si legge più" },
  { cap: "chat", nome: "Chat Teams", nota: "un account Teams senza browser usa questo accesso al posto del suo" },
];

const ORIGINE: Record<NonNullable<Condivisione>["origin"], string> = { mobile: "Outlook Mobile", "outlook-spa": "Outlook web", "teams-spa": "Teams web" };

const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));
const uguali = (a: CapacitaCondivisa[], b: CapacitaCondivisa[]) => a.length === b.length && a.every((c) => b.includes(c));

/**
 * L'account Microsoft condiviso di una casella: spento finché chi la usa non lo attiva qui, con le capacità scelte. Tenant e object id li
 * legge l'agente dal token della casella; ogni capacità si prova con Microsoft prima di restare attiva.
 */
export function CondivisioneMicrosoft({ account }: { account: string }) {
  const [stato, setStato] = useState<Condivisione | undefined>(undefined);
  const [scelte, setScelte] = useState<CapacitaCondivisa[]>(["mail", "calendar"]);
  const [lavoro, setLavoro] = useState(false);
  const [conferma, setConferma] = useState(false);
  const [errore, setErrore] = useState("");

  const leggi = useCallback(async () => {
    try {
      const r = await api("get_condivisione_microsoft");
      if (!r.ok) throw new Error(r.error ?? "Stato della condivisione non letto");
      const mia = r.accounts.find((a) => a.account.toLowerCase() === account.toLowerCase())?.shared ?? null;
      setStato(mia);
      setScelte(mia ? mia.capabilities : ["mail", "calendar"]);
      setErrore("");
    } catch (e) {
      setStato(null);
      setErrore(messaggio(e));
    }
  }, [account]);

  useEffect(() => {
    void leggi();
  }, [leggi]);

  const salva = async (capacita: CapacitaCondivisa[] | null) => {
    setLavoro(true);
    setErrore("");
    try {
      const r = await api("set_condivisione_microsoft", account, capacita);
      if (!r.ok) throw new Error(r.error ?? "Condivisione non cambiata");
      setStato(r.shared ?? null);
      setScelte(r.shared ? r.shared.capabilities : ["mail", "calendar"]);
      setConferma(false);
    } catch (e) {
      setErrore(messaggio(e));
    } finally {
      setLavoro(false);
    }
  };

  if (stato === undefined) return <Spinner className="size-4" />;
  const alterna = (cap: CapacitaCondivisa, si: boolean) => setScelte((s) => (si ? [...s, cap] : s.filter((c) => c !== cap)));
  const cambiate = stato ? !uguali(scelte, stato.capabilities) : true;

  return (
    <div data-condivisione={account} className="flex flex-col gap-2 text-[13px]">
      <p className="text-xs text-muted-foreground">
        {stato
          ? `Attivo, accesso di ${ORIGINE[stato.origin]}: posta, calendario e chat Teams usano questo stesso accesso per le capacità scelte.`
          : "Spento. Attivandolo, questo accesso diventa l'unico per le capacità scelte: nessuna credenziale viene copiata altrove."}
      </p>
      <fieldset className="flex flex-col gap-1.5" disabled={lavoro}>
        <legend className="sr-only">Capacità condivise di {account}</legend>
        {VOCI.map((v) => (
          <label key={v.cap} className="flex items-start gap-2">
            <input type="checkbox" className="mt-0.5" data-capacita={v.cap} checked={scelte.includes(v.cap)} onChange={(e) => alterna(v.cap, e.target.checked)} />
            <span>
              {v.nome} <span className="text-xs text-muted-foreground">{v.nota}</span>
            </span>
          </label>
        ))}
      </fieldset>
      <div className="flex flex-wrap items-center gap-2">
        <Button id="btn-condivisione-salva" size="sm" variant="outline" disabled={lavoro || !scelte.length || !cambiate} onClick={() => void salva(scelte)}>
          {lavoro && <Spinner className="size-3.5" />} {stato ? "Salva capacità" : "Attiva account condiviso"}
        </Button>
        {stato && !conferma && (
          <Button size="sm" variant="ghost" className="text-destructive" disabled={lavoro} onClick={() => setConferma(true)}>
            Spegni
          </Button>
        )}
        {stato && conferma && (
          <span role="alertdialog" className="flex flex-wrap items-center gap-2">
            <span className="text-xs">Spegnere? Gli account Teams che lo usano restano senza accesso.</span>
            <Button id="btn-condivisione-spegni" size="sm" variant="destructive" disabled={lavoro} onClick={() => void salva(null)}>
              Spegni
            </Button>
            <Button size="sm" variant="ghost" disabled={lavoro} onClick={() => setConferma(false)}>
              Annulla
            </Button>
          </span>
        )}
      </div>
      {errore && (
        <p role="alert" className="text-xs text-destructive">
          {errore}
        </p>
      )}
    </div>
  );
}
