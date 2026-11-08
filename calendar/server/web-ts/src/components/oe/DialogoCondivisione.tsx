"use client";

import { BanIcon, CheckIcon, CopyIcon, KeyRoundIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  AVANTI,
  AVVISO_LIVELLO,
  type CondivisionePubblica,
  INDIETRO,
  type Livello,
  NOME_CALENDARIO_MAX,
  NOME_CALENDARIO_PREDEFINITO,
  NOME_MAX,
  quandoLetto,
} from "@/lib/condivisione-comune";
import { api } from "@/lib/oe/api";
import { cn } from "@/lib/utils";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void };

type Campi = { nome: string; livello: Livello; gruppo: string; giorni_indietro: string; giorni_avanti: string; nome_calendario: string };
const VUOTO: Campi = { nome: "", livello: "occupato", gruppo: "", giorni_indietro: String(INDIETRO.predefinito), giorni_avanti: String(AVANTI.predefinito), nome_calendario: "" };
const daLink = (c: CondivisionePubblica): Campi => ({
  nome: c.nome,
  livello: c.livello,
  gruppo: c.ambito === "gruppo" ? c.gruppo : "",
  giorni_indietro: String(c.giorni_indietro),
  giorni_avanti: String(c.giorni_avanti),
  nome_calendario: c.nome_calendario,
});
/** I campi come li vuole il server: i giorni sono numeri se sono cifre, se no si mandano com'è e il server dice cosa non va. */
const perIlServer = (c: Campi) => ({
  nome: c.nome,
  livello: c.livello,
  ambito: c.gruppo ? "gruppo" : "tutti",
  gruppo: c.gruppo,
  giorni_indietro: /^\s*\d+\s*$/.test(c.giorni_indietro) ? Number(c.giorni_indietro) : c.giorni_indietro,
  giorni_avanti: /^\s*\d+\s*$/.test(c.giorni_avanti) ? Number(c.giorni_avanti) : c.giorni_avanti,
  nome_calendario: c.nome_calendario,
});

const TUTTI = "__tutti__";
const LIVELLI_TESTO: { valore: Livello; testo: string }[] = [
  { valore: "occupato", testo: "Occupato" },
  { valore: "titoli", testo: "Con titoli" },
];
type Esito = { testo: string; errore: boolean };
const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));
const giorno = (ms: number) => new Date(ms).toLocaleDateString("it-IT", { day: "2-digit", month: "2-digit", year: "numeric" });

/** Copia un testo negli appunti; se il browser non lo permette (pagina non sicura) prova con una casella di testo. Dice se ci è riuscito. */
async function copia(testo: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(testo);
    return true;
  } catch {
    const t = document.createElement("textarea");
    t.value = testo;
    t.style.position = "fixed";
    t.style.opacity = "0";
    document.body.appendChild(t);
    t.select();
    try {
      return document.execCommand("copy");
    } catch {
      return false;
    } finally {
      t.remove();
    }
  }
}

/** "Condivisione": i link con cui un altro OE Copilot (o qualunque programma che legge un link iCalendar) importa il calendario. */
export function DialogoCondivisione({ aperto, onChiudi }: Props) {
  const { gruppiCalendari } = useOe();
  const [elenco, setElenco] = useState<CondivisionePubblica[] | null>(null);
  const [errore, setErrore] = useState("");
  const [nuovo, setNuovo] = useState<Campi>(VUOTO);
  const [esitoNuovo, setEsitoNuovo] = useState<Esito | null>(null);
  const [evidenziato, setEvidenziato] = useState("");
  const [occupato, setOccupato] = useState(false);
  const [ora, setOra] = useState(() => Date.now());
  const gruppi = Object.keys(gruppiCalendari);

  const carica = useCallback(async () => {
    try {
      const r = await api("get_condivisioni");
      setElenco(r.condivisioni);
      setErrore("");
    } catch (e) {
      setErrore(messaggio(e));
    }
  }, []);

  useEffect(() => {
    if (!aperto) return;
    setNuovo(VUOTO);
    setEsitoNuovo(null);
    setEvidenziato("");
    setOra(Date.now());
    void carica();
    const giro = setInterval(() => setOra(Date.now()), 30_000);
    return () => clearInterval(giro);
  }, [aperto, carica]);

  const crea = async () => {
    setOccupato(true);
    setEsitoNuovo(null);
    try {
      const r = await api("add_condivisione", perIlServer(nuovo));
      if (r.ok) {
        setEvidenziato(r.condivisione?.id ?? "");
        setEsitoNuovo({ testo: `Link creato per ${r.condivisione?.nome}.`, errore: false });
        setNuovo(VUOTO);
        await carica();
      } else setEsitoNuovo({ testo: r.error, errore: true });
    } catch (e) {
      setEsitoNuovo({ testo: messaggio(e), errore: true });
    } finally {
      setOccupato(false);
    }
  };

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-condivisione" className="max-h-[88dvh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Link di condivisione</DialogTitle>
          <DialogDescription>Chi ha il link vede il tuo calendario.</DialogDescription>
        </DialogHeader>

        <section id="cond-elenco" className="flex flex-col gap-2 text-sm">
          <h3 className="font-semibold">I tuoi link</h3>
          {!elenco && !errore && <p className="text-muted-foreground">Carico…</p>}
          {errore && (
            <p role="alert" className="text-destructive">
              {errore}
            </p>
          )}
          {elenco?.length === 0 && <p className="text-xs text-muted-foreground">Nessun link.</p>}
          {elenco?.map((c) => (
            <RigaLink key={c.id} c={c} ora={ora} gruppi={gruppi} evidenziato={evidenziato === c.id} onCambiato={carica} />
          ))}
          {elenco && elenco.length > 0 && <p className="text-xs text-muted-foreground">{elenco.length === 1 ? "1 link" : `${elenco.length} link`}</p>}
        </section>

        <section id="cond-nuovo" className="rounded-lg border p-3 text-sm">
          <h3 className="mb-2 flex items-center gap-2 font-semibold">
            <PlusIcon className="size-4" /> Nuovo link
          </h3>
          <ModuloLink campi={nuovo} onCampi={setNuovo} gruppi={gruppi} prefisso="cond" />
          <div className="mt-3 flex flex-wrap items-center gap-3">
            <Button id="btn-cond-crea" size="sm" disabled={occupato} onClick={() => void crea()}>
              Crea il link
            </Button>
            <span id="cond-esito" role="status" className={cn("text-xs", esitoNuovo?.errore ? "text-destructive" : "text-muted-foreground")}>
              {esitoNuovo?.testo}
            </span>
          </div>
        </section>

        <details id="cond-istruzioni" className="rounded-lg border p-3 text-sm">
          <summary className="cursor-pointer font-medium">Come si importa</summary>
          <ol className="mt-2 list-decimal space-y-1 pl-5 text-xs text-muted-foreground">
            <li>
              Nell&apos;altro OE Copilot: <b>Gestisci calendari</b> → <b>Aggiungi un calendario da link</b>.
            </li>
            <li>
              Incolla il link e premi <b>Aggiungi e verifica</b>.
            </li>
          </ol>
        </details>

        <DialogFooter>
          <Button onClick={onChiudi}>Fine</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** I campi di un link, per crearlo o per cambiarlo. `prefisso` dà gli id dei campi. */
function ModuloLink({ campi, onCampi, gruppi, prefisso }: { campi: Campi; onCampi: (c: Campi) => void; gruppi: string[]; prefisso: string }) {
  const cambia = (parziale: Partial<Campi>) => onCampi({ ...campi, ...parziale });
  // un gruppo scelto prima e sparito dopo resta fra le voci, segnato, così si vede perché il link non serve niente
  const voci = campi.gruppo && !gruppi.includes(campi.gruppo) ? [...gruppi, campi.gruppo] : gruppi;
  return (
    <div className="flex flex-col gap-3">
      <label className="flex flex-col gap-1 text-xs font-medium">
        Per chi è
        <Input id={`${prefisso}-nome`} value={campi.nome} maxLength={NOME_MAX} placeholder="es. Samuele" onChange={(e) => cambia({ nome: e.target.value })} />
      </label>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-medium">Cosa vede</span>
        <div id={`${prefisso}-livello`} role="group" aria-label="Cosa vede" className="inline-flex w-fit rounded-lg border p-0.5">
          {LIVELLI_TESTO.map((l) => (
            <button
              key={l.valore}
              id={`${prefisso}-livello-${l.valore}`}
              type="button"
              data-livello={l.valore}
              aria-pressed={campi.livello === l.valore}
              className={cn("rounded-md px-3 py-1 text-[13px]", campi.livello === l.valore ? "is-active bg-accent font-semibold text-accent-foreground" : "text-muted-foreground hover:bg-muted")}
              onClick={() => cambia({ livello: l.valore })}
            >
              {l.testo}
            </button>
          ))}
        </div>
        <p id={`${prefisso}-avviso`} className={cn("text-xs", campi.livello === "titoli" ? "text-amber-700 dark:text-amber-400" : "text-muted-foreground")}>
          {AVVISO_LIVELLO[campi.livello]}
        </p>
      </div>

      <label className="flex flex-col gap-1 text-xs font-medium">
        Cosa condividere
        <Select value={campi.gruppo || TUTTI} onValueChange={(v) => cambia({ gruppo: v === TUTTI ? "" : v })}>
          <SelectTrigger id={`${prefisso}-ambito`} className="w-full sm:w-72">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={TUTTI}>Tutti i calendari accesi</SelectItem>
            {voci.map((g) => (
              <SelectItem key={g} value={g}>
                Gruppo: {g}
                {gruppi.includes(g) ? "" : " (eliminato)"}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="text-[11px] font-normal text-muted-foreground">
          {campi.gruppo ? "Include anche i calendari spenti." : gruppi.length === 0 ? "I gruppi si creano in Gestisci calendari." : ""}
        </span>
      </label>

      <div className="grid grid-cols-2 gap-3 sm:max-w-sm">
        <label className="flex flex-col gap-1 text-xs font-medium">
          Giorni indietro
          <Input id={`${prefisso}-indietro`} type="number" inputMode="numeric" min={INDIETRO.min} max={INDIETRO.max} value={campi.giorni_indietro} onChange={(e) => cambia({ giorni_indietro: e.target.value })} />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium">
          Giorni avanti
          <Input id={`${prefisso}-avanti`} type="number" inputMode="numeric" min={AVANTI.min} max={AVANTI.max} value={campi.giorni_avanti} onChange={(e) => cambia({ giorni_avanti: e.target.value })} />
        </label>
      </div>

      <label className="flex flex-col gap-1 text-xs font-medium">
        Nome del calendario (facoltativo)
        <Input id={`${prefisso}-nome-calendario`} value={campi.nome_calendario} maxLength={NOME_CALENDARIO_MAX} placeholder={NOME_CALENDARIO_PREDEFINITO} onChange={(e) => cambia({ nome_calendario: e.target.value })} />
      </label>
    </div>
  );
}

type Azione = "ruota" | "revoca" | null;

function RigaLink({ c, ora, gruppi, evidenziato, onCambiato }: { c: CondivisionePubblica; ora: number; gruppi: string[]; evidenziato: boolean; onCambiato: () => Promise<void> }) {
  const [modifica, setModifica] = useState(false);
  const [campi, setCampi] = useState<Campi>(() => daLink(c));
  const [azione, setAzione] = useState<Azione>(null);
  const [esito, setEsito] = useState<Esito | null>(null);
  const [copiato, setCopiato] = useState(false);
  const [occupato, setOccupato] = useState(false);
  const revocato = c.revocato !== null;

  const esegui = async (lavoro: () => Promise<{ ok: boolean; error?: string }>, riuscito: string) => {
    setOccupato(true);
    try {
      const r = await lavoro();
      setEsito({ testo: r.ok ? riuscito : (r.error ?? "Non riuscito."), errore: !r.ok });
      if (r.ok) {
        setAzione(null);
        setModifica(false);
        await onCambiato();
      }
    } catch (e) {
      setEsito({ testo: messaggio(e), errore: true });
    } finally {
      setOccupato(false);
    }
  };

  const copiaUrl = async () => {
    if (!c.url) return;
    const fatto = await copia(c.url);
    setCopiato(fatto);
    setEsito(fatto ? null : { testo: "Copia non riuscita: copialo a mano.", errore: true });
    if (fatto) setTimeout(() => setCopiato(false), 2500);
  };

  return (
    <div data-link={c.id} className={cn("rounded-lg border p-3", revocato && "bg-muted/40 text-muted-foreground", evidenziato && "ring-2 ring-primary")}>
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
        <span className="font-semibold text-foreground">{c.nome}</span>
        <span className="rounded-full border px-2 py-0.5 text-[11px] font-medium" data-livello-link={c.livello}>
          {c.livello === "titoli" ? "Con titoli" : "Occupato"}
        </span>
        <span className="rounded-full border px-2 py-0.5 text-[11px]">{c.ambito === "gruppo" ? `Gruppo: ${c.gruppo}` : "Tutti i calendari"}</span>
        {revocato && <span className="rounded-full bg-destructive/10 px-2 py-0.5 text-[11px] font-medium text-destructive">Revocato il {giorno(c.revocato as number)}</span>}
        {c.gruppo_mancante && <span className="rounded-full bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-400">Gruppo eliminato: link non attivo</span>}
      </div>
      <p className="mt-1 text-xs">
        Ultima lettura: <b data-ultima-lettura>{quandoLetto(c.ultimo_accesso, ora)}</b>
        {c.accessi > 0 && <> · {c.accessi === 1 ? "1 lettura" : `${c.accessi} letture`}</>} · {c.giorni_indietro} giorni indietro, {c.giorni_avanti} avanti
        {c.nome_calendario && <> · si chiama &quot;{c.nome_calendario}&quot;</>}
      </p>

      {c.url && (
        <div className="mt-2 flex items-center gap-1.5">
          <Input readOnly aria-label={`Link di ${c.nome}`} data-url-link value={c.url} className="h-8 min-w-0 flex-1 font-mono text-[11px]" onFocus={(e) => e.currentTarget.select()} />
          <Button type="button" size="sm" variant="outline" data-azione="copia" onClick={() => void copiaUrl()}>
            {copiato ? <CheckIcon /> : <CopyIcon />} {copiato ? "Copiato" : "Copia"}
          </Button>
        </div>
      )}

      {!revocato && !modifica && !azione && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <Button type="button" size="xs" variant="outline" data-azione="modifica" onClick={() => setModifica(true)}>
            <PencilIcon /> Modifica
          </Button>
          <Button type="button" size="xs" variant="outline" data-azione="ruota" onClick={() => setAzione("ruota")}>
            <KeyRoundIcon /> Cambia indirizzo
          </Button>
          <Button type="button" size="xs" variant="outline" data-azione="revoca" className="text-destructive" onClick={() => setAzione("revoca")}>
            <BanIcon /> Revoca
          </Button>
        </div>
      )}
      {revocato && (
        <div className="mt-2">
          <Button type="button" size="xs" variant="outline" data-azione="elimina" disabled={occupato} onClick={() => void esegui(() => api("delete_condivisione", c.id), "")}>
            <Trash2Icon /> Elimina
          </Button>
        </div>
      )}

      {azione && (
        <div role="alertdialog" aria-label="Conferma" data-conferma={azione} className="mt-2 rounded-md border border-destructive/40 bg-destructive/5 p-2 text-xs text-foreground">
          {azione === "revoca" ? (
            <p>
              Revocare il link di <b>{c.nome}</b>? Smette di funzionare subito.
            </p>
          ) : (
            <p>
              Cambiare l&apos;indirizzo del link di <b>{c.nome}</b>? Il vecchio smette di funzionare subito.
            </p>
          )}
          <div className="mt-2 flex gap-2">
            <Button
              type="button"
              size="xs"
              variant="destructive"
              data-azione="conferma"
              disabled={occupato}
              onClick={() =>
                void (azione === "revoca"
                  ? esegui(() => api("revoke_condivisione", c.id), "")
                  : esegui(() => api("rotate_condivisione", c.id), "Indirizzo cambiato."))
              }
            >
              {azione === "revoca" ? "Revoca" : "Cambia indirizzo"}
            </Button>
            <Button type="button" size="xs" variant="outline" data-azione="annulla" onClick={() => setAzione(null)}>
              Annulla
            </Button>
          </div>
        </div>
      )}

      {modifica && (
        <div className="mt-3 rounded-md border p-3 text-foreground">
          <ModuloLink campi={campi} onCampi={setCampi} gruppi={gruppi} prefisso={`mod-${c.id}`} />
          <div className="mt-3 flex gap-2">
            <Button type="button" size="sm" data-azione="salva" disabled={occupato} onClick={() => void esegui(() => api("update_condivisione", c.id, perIlServer(campi)), "Salvato.")}>
              Salva
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              data-azione="annulla-modifica"
              onClick={() => {
                setModifica(false);
                setCampi(daLink(c));
                setEsito(null);
              }}
            >
              Annulla
            </Button>
          </div>
        </div>
      )}

      {esito?.testo && (
        <p data-esito role="status" className={cn("mt-2 text-xs", esito.errore ? "text-destructive" : "text-muted-foreground")}>
          {esito.testo}
        </p>
      )}
    </div>
  );
}
