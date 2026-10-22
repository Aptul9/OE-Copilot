"use client";

import { PlusIcon, RotateCwIcon, Trash2Icon, XIcon } from "lucide-react";
import { Fragment, useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/oe/api";
import { aggiungiGruppo, eliminaGruppo, GRUPPO_NOME_MAX, impostaAppartenenza, rinominaGruppo } from "@/lib/oe/logic";
import type { CalendariRisposta, Calendario, VoceLogo } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { BottoneLogo, PannelloLogo } from "./ControlloLogo";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void };

/** "Calendari da mostrare": un interruttore per calendario, il nome di ogni account, i calendari da link. */
export function DialogoCalendari({ aperto, onChiudi }: Props) {
  const { aggiornaCalendari, ricarica, ricaricaPers, config } = useOe();
  const [dati, setDati] = useState<CalendariRisposta | null>(null);
  // i loghi scelti da qui (quelli di personalizzazione.json non ci sono): nome mostrato -> indirizzo e data
  const [loghi, setLoghi] = useState<Record<string, VoceLogo>>({});
  const [errore, setErrore] = useState("");
  const [nome, setNome] = useState("");
  const [url, setUrl] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);

  const carica = useCallback(
    async (forza: boolean, silenzioso = false) => {
      // silenzioso: l'elenco resta com'è finché non arriva quello nuovo (dopo un cambio di nome)
      if (!silenzioso) setDati(null);
      setErrore("");
      try {
        setDati(await api("get_calendars", forza));
      } catch (e) {
        setErrore(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  const caricaLoghi = useCallback(async () => {
    try {
      setLoghi((await api("get_loghi")).loghi);
    } catch {
      // senza, le scelte non si vedono ma la finestra funziona
    }
  }, []);

  useEffect(() => {
    if (aperto) {
      setEsito("");
      void carica(false);
      void caricaLoghi();
    }
  }, [aperto, carica, caricaLoghi]);

  const dopo = async () => {
    await aggiornaCalendari();
    ricarica();
  };

  // una scelta di logo (o un cambio di nome, che sposta il logo) cambia la mappa che la pagina usa: la si rilegge
  const loghiCambiati = async () => {
    await Promise.all([ricaricaPers(), caricaLoghi()]);
  };

  const aggiungi = async () => {
    if (!url.trim()) {
      setEsito("Manca il link.");
      return;
    }
    setEsito("Verifico il link…");
    setOccupato(true);
    try {
      const r = await api("add_ics_calendar", nome.trim(), url.trim());
      if (r.ok) {
        setEsito("Aggiunto: " + r.feed?.name);
        setNome("");
        setUrl("");
        await carica(false);
        await dopo();
      } else {
        setEsito(r.error ?? "Non riuscito.");
      }
    } catch (e) {
      setEsito(e instanceof Error ? e.message : String(e));
    } finally {
      setOccupato(false);
    }
  };

  // un blocco per nome mostrato: gli account uniti sotto lo stesso nome stanno insieme
  const blocchi: { alias: string; conti: { account: string; cals: Calendario[] }[] }[] = [];
  // acceso o spento lo dice questo dispositivo, non il server
  const spenti = new Set(config.disabled_calendars ?? []);
  for (const letto of dati?.calendars ?? []) {
    const c = { ...letto, enabled: !spenti.has(letto.id) };
    const alias = c.alias || c.account;
    let b = blocchi.find((x) => x.alias === alias);
    if (!b) blocchi.push((b = { alias, conti: [] }));
    const conto = b.conti.find((x) => x.account === c.account);
    if (conto) conto.cals.push(c);
    else b.conti.push({ account: c.account, cals: [c] });
  }
  const accountUnibili = blocchi.flatMap((b) => b.conti.filter((x) => !x.cals[0].is_ics).map((x) => ({ account: x.account, alias: b.alias })));
  // un cambio di nome sposta anche il logo e i gruppi: si rilegge l'elenco della finestra e le scelte
  const dopoRinomina = async () => {
    await dopo();
    await Promise.all([carica(false, true), loghiCambiati()]);
  };

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-calendars" className="max-h-[86dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Calendari da mostrare</DialogTitle>
          <DialogDescription>Accendi o spegni i calendari.</DialogDescription>
        </DialogHeader>

        <GestioneGruppi />
        <UnisciAccount account={accountUnibili} onFatto={dopoRinomina} />

        <div id="cal-list" className="flex flex-col gap-0.5 text-sm">
          {!dati && !errore && (
            <p className="text-muted-foreground">Carico…</p>
          )}
          {errore && <p className="text-destructive">{errore}</p>}
          {dati?.error && <p className="text-destructive">{dati.error}</p>}
          {dati && !dati.error && dati.calendars.length === 0 && (
            <p className="text-muted-foreground">Nessuna casella.</p>
          )}
          {blocchi.map(({ alias, conti }) => (
            <div key={alias} className="mb-2">
              <BloccoAzienda alias={alias} conti={conti} onModificato={dopo} onRicarica={() => carica(false)} onRinominato={dopoRinomina} loghi={loghi} onLoghiCambiati={loghiCambiati} />
            </div>
          ))}
        </div>

        <details className="add-ics rounded-lg border p-3 text-sm">
          <summary className="flex cursor-pointer items-center gap-2 font-medium">
            <PlusIcon className="size-4" /> Aggiungi un calendario da link
          </summary>
          <p className="mt-2 text-xs text-muted-foreground">
            In Google Calendar: <b>Impostazioni</b> → il calendario → <b>Indirizzo segreto in formato iCal</b>.
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
            <RotateCwIcon /> Cerca calendari nuovi
          </Button>
          <Button onClick={onChiudi}>Fine</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type PropsBlocco = {
  alias: string;
  conti: { account: string; cals: Calendario[] }[];
  onModificato: () => Promise<void>;
  onRicarica: () => Promise<void>;
  onRinominato: () => Promise<void>;
  loghi: Record<string, VoceLogo>;
  onLoghiCambiati: () => Promise<void>;
};

function BloccoAzienda({ alias, conti, onModificato, onRicarica, onRinominato, loghi, onLoghiCambiati }: PropsBlocco) {
  const primo = conti[0].cals[0];
  const unito = conti.length > 1;
  const [valore, setValore] = useState(alias);
  const [logoAperto, setLogoAperto] = useState(false);
  useEffect(() => setValore(alias), [alias]);
  const { gruppiCalendari, salvaGruppi, secondari, impostaSecondario } = useOe();
  const nomiGruppi = Object.keys(gruppiCalendari);
  const secondario = secondari.has(alias);
  const salva = async () => {
    if (valore === alias) return;
    for (const { account } of conti) await api("set_account_alias", account, valore);
    await onModificato();
    // il server sposta anche il logo sul nuovo nome: si rilegge l'elenco della finestra (che porta il nome) e le scelte
    await Promise.all([onRinominato(), onLoghiCambiati()]);
  };
  // l'account esce dal nome comune col suo indirizzo come nome, che si cambia subito dopo
  const separa = async (account: string) => {
    await api("set_account_alias", account, account);
    await onModificato();
    await Promise.all([onRinominato(), onLoghiCambiati()]);
  };
  const bottoneLogo = <BottoneLogo nome={alias} colore={primo.color} aperto={logoAperto} onClick={() => setLogoAperto((a) => !a)} />;
  return (
    <>
      <div className="cal-group flex flex-wrap items-center gap-x-2 gap-y-1 py-1">
        <span className="size-2.5 flex-none rounded-full" style={{ background: primo.color }} />
        {primo.is_ics ? (
          <span className="cal-group-name font-semibold">{alias}</span>
        ) : (
          <Input
            className="alias-input h-8 max-w-[220px] font-semibold"
            value={valore}
            data-account={conti[0].account}
            title="Nome da mostrare"
            spellCheck={false}
            onChange={(e) => setValore(e.target.value)}
            onBlur={() => void salva()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        )}
        {bottoneLogo}
        <button
          type="button"
          data-secondario={alias}
          aria-pressed={secondario}
          title={secondario ? "Riporta tra i principali" : "Sposta in Altri calendari"}
          className={cn(
            "rounded-full border px-2 py-0.5 text-[11px] font-medium",
            secondario ? "border-primary bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-background hover:text-foreground",
          )}
          onClick={() => void impostaSecondario(alias, !secondario)}
        >
          Secondario
        </button>
        {/* su uno schermo stretto l'indirizzo va a capo invece di ridursi a una lettera */}
        {!primo.is_ics && !unito && (
          <span className="cal-group-mail min-w-[7rem] flex-1 truncate text-xs text-muted-foreground" title={primo.account}>
            {primo.account}
          </span>
        )}
        {unito && <span className="min-w-[7rem] flex-1 text-xs text-muted-foreground">{conti.length} account uniti</span>}
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
      {logoAperto && <PannelloLogo key={alias} nome={alias} colore={primo.color} voce={loghi[alias]} onCambiato={onLoghiCambiati} />}
      {conti.map(({ account, cals }) => (
        <Fragment key={account}>
          {unito && (
            <div className="flex items-center gap-2 pl-5 pt-1 text-xs text-muted-foreground">
              <span className="min-w-0 flex-1 truncate" title={account}>
                {account}
              </span>
              <button
                type="button"
                data-separa={account}
                title={`Separa ${account} da ${alias}`}
                className="rounded px-1.5 py-0.5 hover:bg-background hover:text-foreground"
                onClick={() => void separa(account)}
              >
                Separa
              </button>
            </div>
          )}
          {cals.map((c) => (
            <RigaCalendario key={c.id} c={c} onModificato={onModificato} onRicarica={onRicarica} />
          ))}
        </Fragment>
      ))}
    </>
  );
}

function RigaCalendario({ c, onModificato, onRicarica }: { c: Calendario; onModificato: () => Promise<void>; onRicarica: () => Promise<void> }) {
  const { accendiCalendario } = useOe();
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
          await accendiCalendario(c.id, e.target.checked);
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

/** Unisce due o più account sotto un nome solo: nella barra laterale sono una voce sola, con un logo e un interruttore. */
function UnisciAccount({ account, onFatto }: { account: { account: string; alias: string }[]; onFatto: () => Promise<void> }) {
  const [nome, setNome] = useState("");
  const [scelti, setScelti] = useState<string[]>([]);
  const [errore, setErrore] = useState("");
  const unisci = async () => {
    if (!nome.trim()) return setErrore("Scrivi un nome.");
    if (!scelti.length) return setErrore("Scegli almeno un account.");
    setErrore("");
    for (const a of scelti) await api("set_account_alias", a, nome.trim());
    setNome("");
    setScelti([]);
    await onFatto();
  };
  return (
    <details id="unisci-account" className="rounded-lg border p-3 text-sm">
      <summary className="cursor-pointer font-medium">Unisci account sotto un nome</summary>
      <p className="mt-1 text-xs text-muted-foreground">Per aggiungere a un nome esistente, scrivi quel nome.</p>
      <ul className="mt-2 flex flex-col gap-0.5">
        {account.map((a) => (
          <li key={a.account}>
            <label className="flex cursor-pointer items-center gap-2.5 rounded-md px-1 py-1 hover:bg-accent">
              <input
                type="checkbox"
                className="size-4 accent-[var(--primary)]"
                data-unisci={a.account}
                checked={scelti.includes(a.account)}
                onChange={(e) => setScelti((s) => (e.target.checked ? [...s, a.account] : s.filter((x) => x !== a.account)))}
              />
              <span className="min-w-0 flex-1 truncate">{a.account}</span>
              <span className="text-xs text-muted-foreground">{a.alias}</span>
            </label>
          </li>
        ))}
      </ul>
      <div className="mt-2 flex items-center gap-2">
        <Input id="unisci-nome" className="h-8 max-w-[220px]" value={nome} placeholder="Nome, es. Personale" onChange={(e) => setNome(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void unisci()} />
        <Button id="unisci-crea" variant="outline" size="sm" onClick={() => void unisci()}>
          Unisci
        </Button>
      </div>
      {errore && <p className="mt-1 text-xs text-destructive">{errore}</p>}
    </details>
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
      <p className="mt-1 text-xs text-muted-foreground">Assegna i calendari con i pulsanti accanto al loro nome.</p>
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
