"use client";

import { PlusIcon, RotateCwIcon, Trash2Icon, XIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/oe/api";
import { aggiungiGruppo, eliminaGruppo, GRUPPO_NOME_MAX, impostaAppartenenza, rinominaGruppo } from "@/lib/oe/logic";
import type { CalendariRisposta, Calendario } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void };

/** "Calendari da mostrare": un interruttore per calendario, il nome di ogni account, i calendari da link. */
export function DialogoCalendari({ aperto, onChiudi }: Props) {
  const { aggiornaCalendari, ricarica } = useOe();
  const [dati, setDati] = useState<CalendariRisposta | null>(null);
  const [errore, setErrore] = useState("");
  const [nome, setNome] = useState("");
  const [url, setUrl] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);

  const carica = useCallback(
    async (forza: boolean) => {
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
            Sono le caselle lette dal server dei calendari: gli accessi si fanno lì.
          </DialogDescription>
        </DialogHeader>

        <GestioneGruppi />

        <div id="cal-list" className="flex flex-col gap-0.5 text-sm">
          {!dati && !errore && (
            <p className="text-muted-foreground">Lettura dal server dei calendari…</p>
          )}
          {errore && <p className="text-destructive">{errore}</p>}
          {dati?.error && <p className="text-destructive">{dati.error}</p>}
          {dati && !dati.error && dati.calendars.length === 0 && (
            <p className="text-muted-foreground">Il server dei calendari non ha caselle da mostrare.</p>
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
  const { gruppiCalendari, salvaGruppi } = useOe();
  const nomiGruppi = Object.keys(gruppiCalendari);
  const salva = async () => {
    if (valore === alias) return;
    await api("set_account_alias", account, valore);
    await onModificato();
  };
  return (
    <>
      <div className="cal-group flex flex-wrap items-center gap-x-2 gap-y-1 py-1">
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
            <span className="cal-group-mail min-w-0 flex-1 truncate text-xs text-muted-foreground" title={primo.account}>
              {primo.account}
            </span>
          </>
        )}
        {nomiGruppi.length > 0 && (
          <span className="ml-auto flex flex-wrap items-center gap-1" role="group" aria-label={`Gruppi di ${alias}`}>
            {nomiGruppi.map((g) => {
              const dentro = gruppiCalendari[g].includes(alias);
              return (
                <button
                  key={g}
                  type="button"
                  data-gruppo={g}
                  data-di={alias}
                  aria-pressed={dentro}
                  title={dentro ? `Togli ${alias} dal gruppo ${g}` : `Metti ${alias} nel gruppo ${g}`}
                  className={cn(
                    "rounded-full border px-2 py-0.5 text-[11px] font-medium",
                    dentro ? "border-primary bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-background hover:text-foreground",
                  )}
                  onClick={() => void salvaGruppi(impostaAppartenenza(gruppiCalendari, g, alias, !dentro))}
                >
                  {g}
                </button>
              );
            })}
          </span>
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

/** Crea, rinomina ed elimina i gruppi. L'appartenenza si sceglie sull'intestazione di ogni calendario. */
function GestioneGruppi() {
  const { gruppiCalendari, salvaGruppi } = useOe();
  const [nuovo, setNuovo] = useState("");
  const [errore, setErrore] = useState("");
  const crea = async () => {
    const r = aggiungiGruppo(gruppiCalendari, nuovo);
    if ("errore" in r) return setErrore(r.errore);
    setErrore("");
    setNuovo("");
    await salvaGruppi(r.gruppi);
  };
  return (
    <section id="gruppi-gestione" className="rounded-lg border p-3 text-sm">
      <h3 className="font-medium">Gruppi</h3>
      <p className="mt-1 text-xs text-muted-foreground">
        Un calendario può stare in più gruppi: si sceglie con i pulsanti accanto al suo nome. Nella barra laterale un gruppo mostra solo i suoi calendari.
      </p>
      <ul className="mt-2 flex flex-col gap-1.5">
        {Object.keys(gruppiCalendari).map((nome) => (
          <RigaGruppo key={nome} nome={nome} />
        ))}
      </ul>
      <div className="mt-2 flex items-center gap-2">
        <Input
          id="gruppo-nuovo"
          className="h-8 max-w-[220px]"
          value={nuovo}
          maxLength={GRUPPO_NOME_MAX}
          placeholder="Nuovo gruppo"
          onChange={(e) => setNuovo(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void crea()}
        />
        <Button id="gruppo-crea" variant="outline" size="sm" onClick={() => void crea()}>
          <PlusIcon /> Crea
        </Button>
      </div>
      {errore && <p className="mt-1 text-xs text-destructive">{errore}</p>}
    </section>
  );
}

function RigaGruppo({ nome }: { nome: string }) {
  const { gruppiCalendari, salvaGruppi } = useOe();
  const [valore, setValore] = useState(nome);
  const [errore, setErrore] = useState("");
  useEffect(() => setValore(nome), [nome]);
  const quanti = gruppiCalendari[nome]?.length ?? 0;
  const rinomina = async () => {
    if (valore === nome) return;
    const r = rinominaGruppo(gruppiCalendari, nome, valore);
    if ("errore" in r) {
      setErrore(r.errore);
      setValore(nome);
      return;
    }
    setErrore("");
    await salvaGruppi(r.gruppi);
  };
  return (
    <li className="flex flex-wrap items-center gap-x-2 gap-y-0.5" data-riga-gruppo={nome}>
      <Input
        className="h-8 min-w-0 flex-1 font-medium sm:max-w-[220px] sm:flex-none"
        value={valore}
        maxLength={GRUPPO_NOME_MAX}
        spellCheck={false}
        aria-label={`Nome del gruppo ${nome}`}
        onChange={(e) => setValore(e.target.value)}
        onBlur={() => void rinomina()}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
      <span className="flex-none text-xs text-muted-foreground">{quanti === 1 ? "1 calendario" : `${quanti} calendari`}</span>
      <button
        type="button"
        className="flex-none rounded p-1 text-muted-foreground hover:bg-accent hover:text-destructive sm:ml-auto"
        title={`Elimina il gruppo ${nome}`}
        data-elimina-gruppo={nome}
        onClick={() => void salvaGruppi(eliminaGruppo(gruppiCalendari, nome))}
      >
        <Trash2Icon className="size-3.5" />
      </button>
      {errore && <span className="basis-full text-xs text-destructive">{errore}</span>}
    </li>
  );
}
