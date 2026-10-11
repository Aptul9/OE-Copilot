"use client";

import { PlusIcon, RotateCwIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/oe/api";
import type { CalendariRisposta, Calendario } from "@/lib/oe/types";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void };

/** "Calendari da mostrare": un interruttore per calendario, il nome di ogni account, i calendari da link. */
export function DialogoCalendari({ aperto, onChiudi }: Props) {
  const { pers, aggiornaCalendari, ricarica } = useOe();
  const [dati, setDati] = useState<CalendariRisposta | null>(null);
  const [errore, setErrore] = useState("");
  const [forzata, setForzata] = useState(false);
  const [nome, setNome] = useState("");
  const [url, setUrl] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);
  const remota = pers.sorgente === "remota";

  const carica = useCallback(
    async (forza: boolean) => {
      setForzata(forza);
      setDati(null);
      setErrore("");
      try {
        setDati(await api("get_calendars", forza));
      } catch (e) {
        setErrore(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  useEffect(() => {
    if (aperto) {
      setEsito("");
      void carica(false);
    }
  }, [aperto, carica]);

  const dopo = async () => {
    await aggiornaCalendari();
    ricarica();
  };

  const aggiungi = async () => {
    if (!url.trim()) {
      setEsito("Incolla prima il link.");
      return;
    }
    setEsito("Verifico il link…");
    setOccupato(true);
    try {
      const r = await api("add_ics_calendar", nome.trim(), url.trim());
      if (r.ok) {
        setEsito("✅ Aggiunto: " + r.feed?.name);
        setNome("");
        setUrl("");
        await carica(false);
        await dopo();
      } else {
        setEsito("⚠ " + r.error);
      }
    } catch (e) {
      setEsito("⚠ " + (e instanceof Error ? e.message : String(e)));
    } finally {
      setOccupato(false);
    }
  };

  const gruppi: { account: string; cals: Calendario[] }[] = [];
  for (const c of dati?.calendars ?? []) {
    const ultimo = gruppi[gruppi.length - 1];
    if (ultimo && ultimo.account === c.account) ultimo.cals.push(c);
    else gruppi.push({ account: c.account, cals: [c] });
  }

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-calendars" className="max-h-[86dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Calendari da mostrare</DialogTitle>
          <DialogDescription>
            {remota
              ? "Sono le caselle lette dal server dei calendari: gli accessi si fanno lì."
              : "Sono i calendari già presenti nel profilo Outlook: nessun accesso da fare."}
          </DialogDescription>
        </DialogHeader>

        <div id="cal-list" className="flex flex-col gap-0.5 text-sm">
          {!dati && !errore && (
            <p className="text-muted-foreground">{remota ? "Lettura dal server dei calendari…" : forzata ? "Ricerca completa in Outlook, richiede qualche secondo…" : "Lettura da Outlook…"}</p>
          )}
          {errore && <p className="text-destructive">{errore}</p>}
          {dati?.error && <p className="text-destructive">{dati.error}</p>}
          {dati && !dati.error && dati.calendars.length === 0 && (
            <p className="text-muted-foreground">{remota ? "Il server dei calendari non ha caselle da mostrare." : "Nessun calendario trovato nel profilo Outlook."}</p>
          )}
          {gruppi.map(({ account, cals }) => (
            <div key={account} className="mb-2">
              <GruppoAccount cals={cals} account={account} onModificato={dopo} onRicarica={() => carica(false)} />
            </div>
          ))}
        </div>

        <details className="add-ics rounded-lg border p-3 text-sm">
          <summary className="flex cursor-pointer items-center gap-2 font-medium">
            <PlusIcon className="size-4" /> Aggiungi un calendario da link (Google, iCal…)
          </summary>
          <p className="mt-2 text-xs text-muted-foreground">
            In Google Calendar: <b>Impostazioni</b> → scegli il calendario → <b>Indirizzo segreto in formato iCal</b> → copia il link e incollalo qui. Il link vale come una chiave: chi ce l&apos;ha legge il
            calendario, quindi resta solo sul server.
          </p>
          <label className="mt-2 flex flex-col gap-1 text-xs font-medium">
            Nome da mostrare
            <Input id="ics-name" value={nome} onChange={(e) => setNome(e.target.value)} placeholder="es. Azienda (Google)" />
          </label>
          <label className="mt-2 flex flex-col gap-1 text-xs font-medium">
            Link ICS
            <Input id="ics-url" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://calendar.google.com/calendar/ical/.../basic.ics" />
          </label>
          <div className="mt-3 flex items-center gap-3">
            <Button id="btn-add-ics" variant="outline" size="sm" disabled={occupato} onClick={() => void aggiungi()}>
              Aggiungi e verifica
            </Button>
            <span id="ics-result" className="text-xs text-muted-foreground">
              {esito}
            </span>
          </div>
        </details>

        <DialogFooter className="sm:justify-between">
          <Button id="btn-rescan" variant="outline" onClick={() => void carica(true)}>
            <RotateCwIcon /> Ricerca calendari nuovi
          </Button>
          <Button onClick={onChiudi}>Fine</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function GruppoAccount({ cals, account, onModificato, onRicarica }: { cals: Calendario[]; account: string; onModificato: () => Promise<void>; onRicarica: () => Promise<void> }) {
  const primo = cals[0];
  const alias = primo.alias || primo.account;
  const [valore, setValore] = useState(alias);
  useEffect(() => setValore(alias), [alias]);
  const salva = async () => {
    if (valore === alias) return;
    await api("set_account_alias", account, valore);
    await onModificato();
  };
  return (
    <>
      <div className="cal-group flex items-center gap-2 py-1">
        <span className="size-2.5 flex-none rounded-full" style={{ background: primo.color }} />
        {primo.is_ics ? (
          <span className="cal-group-name font-semibold">{alias}</span>
        ) : (
          <>
            <Input
              className="alias-input h-8 max-w-[220px] font-semibold"
              value={valore}
              data-account={account}
              title="Come vuoi chiamare questo account"
              spellCheck={false}
              onChange={(e) => setValore(e.target.value)}
              onBlur={() => void salva()}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
            <span className="cal-group-mail min-w-0 truncate text-xs text-muted-foreground" title={primo.account}>
              {primo.account}
            </span>
          </>
        )}
      </div>
      {cals.map((c) => (
        <RigaCalendario key={c.id} c={c} onModificato={onModificato} onRicarica={onRicarica} />
      ))}
    </>
  );
}

function RigaCalendario({ c, onModificato, onRicarica }: { c: Calendario; onModificato: () => Promise<void>; onRicarica: () => Promise<void> }) {
  const [acceso, setAcceso] = useState(c.enabled);
  useEffect(() => setAcceso(c.enabled), [c.enabled]);
  const nome = c.label.startsWith(c.account + " · ") ? c.label.slice(c.account.length + 3) : c.label;
  const rimovibile = !!c.is_ics && !c.builtin;   // le festività sono incluse: non si tolgono come un link
  const coda = c.builtin ? "incluso" : c.is_ics ? "da link" : c.count == null ? c.nota || "" : `${c.count} elementi`;
  return (
    <label className="cal-row flex cursor-pointer items-center gap-2.5 rounded-md py-1 pl-5 pr-1 hover:bg-accent">
      <input
        type="checkbox"
        className="size-4 accent-[var(--primary)]"
        data-id={c.id}
        checked={acceso}
        onChange={async (e) => {
          setAcceso(e.target.checked);
          await api("set_calendar_enabled", c.id, e.target.checked);
          await onModificato();
        }}
      />
      <span className="min-w-0 flex-1 truncate">{nome}</span>
      <span className="cal-count text-xs text-muted-foreground">{coda}</span>
      {rimovibile && (
        <button
          className="cal-rm rounded p-1 text-muted-foreground hover:bg-background hover:text-destructive"
          title="Rimuovi questo calendario"
          data-rm={c.id}
          onClick={async (e) => {
            e.preventDefault();
            await api("remove_ics_calendar", c.id);
            await onRicarica();
            await onModificato();
          }}
        >
          <XIcon className="size-3.5" />
        </button>
      )}
    </label>
  );
}
