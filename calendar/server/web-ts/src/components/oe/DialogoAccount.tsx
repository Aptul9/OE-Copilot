"use client";

import {
  ArrowLeftIcon,
  BellIcon,
  CalendarPlusIcon,
  ChevronRightIcon,
  EyeOffIcon,
  LinkIcon,
  LogOutIcon,
  PaletteIcon,
  PlusIcon,
  RotateCwIcon,
  Settings2Icon,
  Share2Icon,
  Trash2Icon,
  UsersIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useInfoApp } from "@/components/ServerApp";
import { Button, buttonVariants } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import type { Modo2fa } from "@/lib/config";
import { esci } from "@/lib/esci";
import { api } from "@/lib/oe/api";
import {
  aggiungiGruppo,
  dataEvento,
  eliminaGruppo,
  eliminatiRecenti,
  eliminatoDa,
  fmtRange,
  scadeTra,
  GIORNI_RIPRISTINO,
  GRUPPO_NOME_MAX,
  impostaAppartenenza,
  mappaLoghi,
  rinominaGruppo,
} from "@/lib/oe/logic";
import type { Calendario, CasellaAccesso, TemaConfig, VoceLogo } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { CondivisioneMicrosoft } from "./CondivisioneMicrosoft";
import { AvatarLogo, CerchioLogo } from "./ControlloLogo";
import { ImpostazioniCondivisione } from "./ImpostazioniCondivisione";
import { ImpostazioniPromemoria } from "./ImpostazioniPromemoria";
import { useOe } from "./OeProvider";

// Due finestre con la stessa forma, come le impostazioni di Outlook: a sinistra le voci, a destra quella scelta; su uno schermo stretto una
// colonna alla volta. "Account" (pulsante nella barra): le caselle con Aggiungi account, i calendari da link, i gruppi, gli eliminati.
// "Impostazioni" (ingranaggio): aspetto, promemoria, condivisione, generale.

/** `account`: caselle, calendari da link, gruppi, eliminati (pulsante Account). `impostazioni`: aspetto, promemoria, condivisione, generale (ingranaggio). */
type Props = { modo: "account" | "impostazioni"; aperto: boolean; onChiudi: () => void; accesso2fa?: Modo2fa };
type Provider = "microsoft" | "google";

/** I marchi dei fornitori, nei loro colori: quattro quadrati Microsoft, la G di Google. */
export function IconaMicrosoft({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 21 21" aria-hidden className={className}>
      <rect x="1" y="1" width="9" height="9" fill="#f25022" />
      <rect x="11" y="1" width="9" height="9" fill="#7fba00" />
      <rect x="1" y="11" width="9" height="9" fill="#00a4ef" />
      <rect x="11" y="11" width="9" height="9" fill="#ffb900" />
    </svg>
  );
}
export function IconaGoogle({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 48 48" aria-hidden className={className}>
      <path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z" />
      <path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z" />
      <path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z" />
      <path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z" />
    </svg>
  );
}
const IconaFornitore = ({ provider, className }: { provider: Provider; className?: string }) =>
  provider === "google" ? <IconaGoogle className={className} /> : <IconaMicrosoft className={className} />;

const TEMI: { valore: TemaConfig; testo: string }[] = [
  { valore: "auto", testo: "Automatico" },
  { valore: "light", testo: "Chiaro" },
  { valore: "dark", testo: "Scuro" },
];
const INTERVALLI = [
  { valore: 30, testo: "30 secondi" },
  { valore: 60, testo: "1 minuto" },
  { valore: 120, testo: "2 minuti" },
  { valore: 300, testo: "5 minuti" },
];
const IMPOSTAZIONI = [
  { chiave: "aspetto", testo: "Aspetto", icona: PaletteIcon },
  { chiave: "promemoria", testo: "Promemoria", icona: BellIcon },
  { chiave: "condivisione", testo: "Condivisione", icona: Share2Icon },
  { chiave: "generale", testo: "Generale", icona: Settings2Icon },
] as const;
const PALLINO: Record<string, string> = { ok: "bg-emerald-600", avviso: "bg-amber-500", errore: "bg-red-600" };

/** L'id del calendario di una casella in get_calendars (idCalendario in lib/core/agente.ts). */
const idCasella = (account: string) => "remoto::" + account.toLowerCase();
const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));
const statoDi = (c: CasellaAccesso) => (c.scaduto ? "errore" : c.stato in PALLINO ? c.stato : "ok");
/** Le aziende inoltrate in una casella (get_stato_caselle), come calendari di get_calendars. */
const calendariInoltrati = (c: CasellaAccesso, calendari: Calendario[]) =>
  (c.inoltrate ?? []).flatMap((n) => calendari.filter((x) => x.is_inoltro && (x.alias || x.account) === n));

export function DialogoAccount({ modo, aperto, onChiudi, accesso2fa = "off" }: Props) {
  const oe = useOe();
  const conAccount = modo === "account";
  const [caselle, setCaselle] = useState<CasellaAccesso[] | null>(null);
  const [erroreCaselle, setErroreCaselle] = useState("");
  // vuota: la prima casella
  const [sezione, setSezione] = useState("");
  // schermo stretto: false = l'elenco, true = la sezione scelta
  const [dettaglio, setDettaglio] = useState(false);
  const [loghi, setLoghi] = useState<Record<string, VoceLogo>>({});
  const [appenaEntrato, setAppenaEntrato] = useState("");
  // le voci con la freccia aperta: "casella:<indirizzo>" e "link"
  const [aperteNav, setAperteNav] = useState<ReadonlySet<string>>(new Set());

  const caricaCaselle = useCallback(async () => {
    try {
      const r = await api("get_stato_caselle");
      setCaselle(r.caselle ?? []);
      setErroreCaselle(r.ok ? "" : (r.error ?? "Caselle non lette"));
    } catch (e) {
      setCaselle([]);
      setErroreCaselle(messaggio(e));
    }
  }, []);
  const caricaLoghi = useCallback(async () => {
    try {
      setLoghi((await api("get_loghi")).loghi);
    } catch {
      // senza, il pannello del logo non mostra l'indirizzo scelto ma funziona
    }
  }, []);
  useEffect(() => {
    if (!aperto) return;
    setSezione("");
    setDettaglio(false);
    setAppenaEntrato("");
    setAperteNav(new Set());
    if (!conAccount) return;
    void caricaCaselle();
    void caricaLoghi();
  }, [aperto, conAccount, caricaCaselle, caricaLoghi]);

  const dopoAccesso = async (account: string) => {
    await caricaCaselle();
    await oe.aggiornaCalendari();
    oe.ricarica();
    setAppenaEntrato(account.toLowerCase());
    setSezione("casella:" + account.toLowerCase());
  };
  const acc = useAccesso(aperto, dopoAccesso);

  const loghiCambiati = async () => {
    await Promise.all([oe.ricaricaPers(), caricaLoghi()]);
  };

  // le caselle nell'ordine della barra laterale (ordine_calendari), quelle appena aggiunte in fondo
  const ordinate = useMemo(() => {
    const pos = (a: string) => {
      const i = oe.calendari.findIndex((c) => c.id === idCasella(a));
      return i < 0 ? Number.MAX_SAFE_INTEGER : i;
    };
    return [...(caselle ?? [])].sort((a, b) => pos(a.account) - pos(b.account));
  }, [caselle, oe.calendari]);
  const nomeDi = (c: CasellaAccesso) => oe.calendari.find((x) => x.id === idCasella(c.account))?.alias || c.nome || c.account;
  const coloreDi = (c: CasellaAccesso) => oe.calendari.find((x) => x.id === idCasella(c.account))?.color ?? "#64748b";
  const loghiPagina = useMemo(() => mappaLoghi(oe.pers.interfaccia.loghi), [oe.pers]);

  // i calendari sotto ogni voce con la freccia: le aziende inoltrate in una casella, i calendari da link e personali
  const figli = useMemo(() => {
    const m = new Map<string, Calendario[]>();
    for (const c of ordinate) m.set("casella:" + c.account.toLowerCase(), calendariInoltrati(c, oe.calendari));
    m.set("link", oe.calendari.filter((c) => c.is_ics || c.is_local));
    return m;
  }, [ordinate, oe.calendari]);

  const attiva = sezione || (!conAccount ? "aspetto" : ordinate[0] ? "casella:" + ordinate[0].account.toLowerCase() : caselle ? "aggiungi" : "");
  const casellaAttiva = attiva.startsWith("casella:") ? ordinate.find((c) => "casella:" + c.account.toLowerCase() === attiva) : undefined;
  const calendarioAttivo = attiva.startsWith("calendario:") ? oe.calendari.find((c) => "calendario:" + c.id === attiva) : undefined;
  const nEliminati = eliminatiRecenti(oe.config.eventi_eliminati, Date.now()).length;
  const nGruppi = Object.keys(oe.gruppiCalendari).length;

  const scegli = (k: string) => {
    setSezione(k);
    setDettaglio(true);
  };
  const alterna = (k: string, apri = !aperteNav.has(k)) =>
    setAperteNav((s) => {
      const n = new Set(s);
      if (apri) n.add(k);
      else n.delete(k);
      return n;
    });
  const rimossa = async () => {
    await caricaCaselle();
    await oe.aggiornaCalendari();
    oe.ricarica();
    setSezione("");
  };

  let pannello: ReactNode;
  if (!attiva) pannello = <p className="text-muted-foreground">Carico…</p>;
  else if (casellaAttiva)
    pannello = (
      <PannelloCasella
        key={casellaAttiva.account}
        casella={casellaAttiva}
        nome={nomeDi(casellaAttiva)}
        colore={coloreDi(casellaAttiva)}
        uniti={ordinate.filter((c) => c !== casellaAttiva && nomeDi(c) === nomeDi(casellaAttiva)).map((c) => c.account)}
        appenaEntrato={appenaEntrato === casellaAttiva.account.toLowerCase()}
        acc={acc}
        loghi={loghi}
        onLoghiCambiati={loghiCambiati}
        onRimossa={rimossa}
        onApri={(k) => {
          alterna("casella:" + casellaAttiva.account.toLowerCase(), true);
          scegli(k);
        }}
      />
    );
  else if (calendarioAttivo) {
    const casa = ordinate.find((c) => figli.get("casella:" + c.account.toLowerCase())?.includes(calendarioAttivo));
    pannello = (
      <PannelloCalendario
        key={calendarioAttivo.id}
        c={calendarioAttivo}
        sotto={
          casa
            ? `Copie inoltrate in ${nomeDi(casa)}`
            : calendarioAttivo.builtin
              ? "Inclusa"
              : calendarioAttivo.is_ics
                ? "Da un link ICS"
                : "Eventi creati qui"
        }
        loghi={loghi}
        onLoghiCambiati={loghiCambiati}
        onRimosso={() => setSezione("link")}
      />
    );
  } else if (attiva === "aggiungi") pannello = <PannelloAggiungi acc={acc} onLink={() => scegli("link")} />;
  else if (attiva === "link")
    pannello = (
      <PannelloLink
        onAggiunto={() => alterna("link", true)}
        onApri={(k) => {
          alterna("link", true);
          scegli(k);
        }}
      />
    );
  else if (attiva === "gruppi") pannello = <PannelloGruppi />;
  else if (attiva === "eliminati") pannello = <PannelloEliminati />;
  else if (attiva === "aspetto") pannello = <PannelloAspetto />;
  else if (attiva === "promemoria") pannello = <PannelloIncluso titolo="Promemoria"><ImpostazioniPromemoria /></PannelloIncluso>;
  else if (attiva === "condivisione") pannello = <PannelloIncluso titolo="Condivisione"><ImpostazioniCondivisione /></PannelloIncluso>;
  else if (attiva === "generale") pannello = <PannelloGenerale accesso2fa={accesso2fa} />;
  else pannello = <p className="text-muted-foreground">Casella non trovata.</p>;

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent
        id={conAccount ? "modal-account" : "modal-appsettings"}
        // il primo elemento sarebbe la freccia "Rileggi", con l'anello del fuoco: si parte senza fuoco su un controllo
        onOpenAutoFocus={(e) => e.preventDefault()}
        className={cn(
          "flex flex-col gap-0 overflow-hidden p-0 max-md:h-dvh max-md:max-w-full max-md:rounded-none",
          conAccount ? "h-[min(46rem,92dvh)] sm:max-w-5xl" : "h-[min(38rem,92dvh)] sm:max-w-4xl",
        )}
      >
        <DialogHeader className="border-b px-4 py-3">
          <DialogTitle>{conAccount ? "Account" : "Impostazioni"}</DialogTitle>
          <DialogDescription className="sr-only">{conAccount ? "Caselle, calendari da link, gruppi ed eventi eliminati" : "Aspetto, promemoria, condivisione e generale"}</DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-1">
          <nav id="account-nav" aria-label="Sezioni" className={cn("scroll-thin w-full flex-none overflow-y-auto bg-sidebar p-2 md:w-72 md:border-r", dettaglio && "max-md:hidden")}>
            {!conAccount &&
              IMPOSTAZIONI.map(({ chiave, testo, icona: Icona }) => (
                <VoceNav key={chiave} chiave={chiave} attiva={attiva} onScegli={scegli} id={`nav-${chiave}`}>
                  <Icona className="mx-1 size-4 text-muted-foreground" /> {testo}
                </VoceNav>
              ))}
            {conAccount && (
              <>
                <div className="flex items-center justify-between px-2 pb-1 pt-1">
                  <TitoloGruppo>Account</TitoloGruppo>
                  <button
                    id="btn-rescan"
                    type="button"
                    className="rounded-md p-2 text-muted-foreground hover:bg-accent hover:text-foreground"
                    title="Rileggi le caselle e cerca calendari nuovi"
                    aria-label="Rileggi le caselle"
                    onClick={() => void Promise.all([caricaCaselle(), oe.aggiornaCalendari(true)])}
                  >
                    <RotateCwIcon className="size-3.5" />
                  </button>
                </div>
                {caselle === null && <p className="px-2 py-1.5 text-xs text-muted-foreground">Carico…</p>}
                {erroreCaselle && <p className="px-2 py-1.5 text-xs text-destructive">{erroreCaselle}</p>}
                {ordinate.map((c) => {
                  const nome = nomeDi(c);
                  const stato = statoDi(c);
                  const k = "casella:" + c.account.toLowerCase();
                  return (
                    <Ramo key={c.account} nome={nome} figli={figli.get(k) ?? []} aperto={aperteNav.has(k)} onAlterna={() => alterna(k)} attiva={attiva} onScegli={scegli} loghi={loghiPagina}>
                      <VoceNav chiave={k} attiva={attiva} onScegli={scegli} dati={{ "data-casella": c.account }} className="min-w-0 flex-1 pl-1">
                        <CerchioLogo nome={nome} colore={coloreDi(c)} src={loghiPagina[nome]} lato={26} />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate">{nome}</span>
                          <span className="truncate text-[11px] font-normal text-muted-foreground">{c.account}</span>
                        </span>
                        <span className={cn("size-2 flex-none rounded-full", PALLINO[stato])} title={c.scaduto ? "Accesso scaduto" : c.msg || "Connesso"} />
                        <span className="sr-only">{c.scaduto ? "accesso scaduto" : stato === "ok" ? "connesso" : "da controllare"}</span>
                      </VoceNav>
                    </Ramo>
                  );
                })}
                <VoceNav chiave="aggiungi" attiva={attiva} onScegli={scegli} id="nav-aggiungi-account" className="pl-8 text-primary">
                  <PlusIcon className="mx-1 size-4" /> Aggiungi account
                </VoceNav>

                <TitoloGruppo className="px-2 pb-1 pt-4">Calendari</TitoloGruppo>
                <Ramo nome="Da link e personali" figli={figli.get("link") ?? []} aperto={aperteNav.has("link")} onAlterna={() => alterna("link")} attiva={attiva} onScegli={scegli} loghi={loghiPagina}>
                  <VoceNav chiave="link" attiva={attiva} onScegli={scegli} id="nav-link" className="min-w-0 flex-1 pl-1">
                    <LinkIcon className="mx-1 size-4 text-muted-foreground" /> Da link e personali
                  </VoceNav>
                </Ramo>
                <VoceNav chiave="gruppi" attiva={attiva} onScegli={scegli} conteggio={nGruppi} id="nav-gruppi" className="pl-8">
                  <UsersIcon className="mx-1 size-4 text-muted-foreground" /> Gruppi
                </VoceNav>
                <VoceNav chiave="eliminati" attiva={attiva} onScegli={scegli} conteggio={nEliminati} id="nav-eliminati" className="pl-8">
                  <Trash2Icon className="mx-1 size-4 text-muted-foreground" /> Eliminati
                </VoceNav>
              </>
            )}
          </nav>
          <section id="account-pannello" className={cn("scroll-thin min-w-0 flex-1 overflow-y-auto px-5 py-4 text-sm", !dettaglio && "max-md:hidden")}>
            <button type="button" className="mb-3 flex items-center gap-1.5 text-[13px] font-medium text-primary md:hidden" onClick={() => setDettaglio(false)}>
              <ArrowLeftIcon className="size-4" /> Indietro
            </button>
            {pannello}
          </section>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function TitoloGruppo({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("text-[11px] font-semibold uppercase tracking-wide text-muted-foreground", className)}>{children}</div>;
}

function VoceNav({
  chiave,
  attiva,
  onScegli,
  conteggio,
  id,
  className,
  dati,
  children,
}: {
  chiave: string;
  attiva: string;
  onScegli: (k: string) => void;
  conteggio?: number;
  id?: string;
  className?: string;
  dati?: Record<string, string>;
  children: ReactNode;
}) {
  const corrente = attiva === chiave;
  return (
    <button
      type="button"
      id={id}
      {...dati}
      aria-current={corrente ? "page" : undefined}
      className={cn(
        "flex min-h-9 w-full items-center gap-2 rounded-md px-2 py-1 text-left text-[13px] hover:bg-accent",
        corrente && "bg-accent font-semibold text-accent-foreground",
        className,
      )}
      onClick={() => onScegli(chiave)}
    >
      {children}
      {!!conteggio && <span className="ml-auto text-[11px] font-normal text-muted-foreground">{conteggio}</span>}
    </button>
  );
}

/**
 * Una voce dell'elenco con i suoi calendari sotto, come le cartelle di Outlook: la freccia a sinistra li mostra o li nasconde, ognuno apre la
 * sua pagina. Senza calendari al posto della freccia c'è uno spazio, così le voci restano in colonna.
 */
function Ramo({
  nome,
  figli,
  aperto,
  onAlterna,
  attiva,
  onScegli,
  loghi,
  children,
}: {
  nome: string;
  figli: Calendario[];
  aperto: boolean;
  onAlterna: () => void;
  attiva: string;
  onScegli: (k: string) => void;
  loghi: Record<string, string>;
  children: ReactNode;
}) {
  return (
    <div data-ramo={nome}>
      <div className="flex items-center">
        {figli.length ? (
          <button
            type="button"
            data-freccia={nome}
            aria-expanded={aperto}
            aria-label={`${aperto ? "Nascondi" : "Mostra"} i calendari di ${nome}`}
            title={`${aperto ? "Nascondi" : "Mostra"} i calendari (${figli.length})`}
            className="grid h-9 w-7 flex-none place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
            onClick={onAlterna}
          >
            <ChevronRightIcon className={cn("size-4 transition-transform", aperto && "rotate-90")} />
          </button>
        ) : (
          <span className="w-7 flex-none" />
        )}
        {children}
      </div>
      {aperto && figli.length > 0 && (
        <ul role="group" aria-label={`Calendari di ${nome}`} className="mb-1 ml-[1.375rem] border-l pl-2">
          {figli.map((c) => {
            const n = c.alias || c.account;
            return (
              <li key={c.id}>
                <VoceNav chiave={"calendario:" + c.id} attiva={attiva} onScegli={onScegli} dati={{ "data-calendario": c.id }} className="min-h-8 py-0.5">
                  <CerchioLogo nome={n} colore={c.color} src={loghi[n]} lato={20} />
                  <span className={cn("min-w-0 flex-1 truncate", !c.enabled && "text-muted-foreground")}>{n}</span>
                  {!c.enabled && (
                    <span title="Nascosto su questo dispositivo" className="flex-none text-muted-foreground">
                      <EyeOffIcon className="size-3.5" />
                      <span className="sr-only">nascosto su questo dispositivo</span>
                    </span>
                  )}
                </VoceNav>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function Titolo({ children, sotto }: { children: ReactNode; sotto?: ReactNode }) {
  return (
    <div className="mb-4">
      <h2 className="text-lg font-semibold">{children}</h2>
      {sotto && <p className="mt-0.5 text-xs text-muted-foreground">{sotto}</p>}
    </div>
  );
}

/** Una riga etichetta e controlli: affiancati sullo schermo largo, uno sotto l'altro su quello stretto. */
function Riga({ etichetta, children }: { etichetta: string; children: ReactNode }) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-[8.5rem_1fr] sm:items-start sm:gap-3">
      <span className="pt-1 text-xs font-medium text-muted-foreground">{etichetta}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

// ---- Accesso a una casella: codice Microsoft che si completa da solo, o indirizzo finale da incollare ----------------
type StatoAccesso = {
  provider: Provider;
  /** Vuoto: un account nuovo. */
  email: string;
  fase: "avvio" | "codice" | "incolla";
  codice?: string;
  /** La pagina dove si accede adesso. */
  url?: string;
  /** Il link diretto Microsoft, se il codice non va. */
  urlPkce?: string;
  state: string;
  esito: string;
  inCorso: boolean;
};

function useAccesso(aperto: boolean, onFatto: (account: string) => Promise<void>) {
  const [a, setA] = useState<StatoAccesso | null>(null);
  const fatto = useRef(onFatto);
  fatto.current = onFatto;
  useEffect(() => {
    if (!aperto) setA(null);
  }, [aperto]);

  const avvia = async (provider: Provider, email = "") => {
    setA({ provider, email, fase: "avvio", state: "", esito: "", inCorso: true });
    try {
      const r = await api("inizia_accesso_microsoft", email, provider, provider === "microsoft" ? "device" : "pkce");
      if (!r.ok || !r.url) {
        setA({ provider, email, fase: "avvio", state: "", esito: r.error ?? "Accesso non avviato.", inCorso: false });
        return;
      }
      if (r.user_code) navigator.clipboard?.writeText(r.user_code).catch(() => {});
      window.open(r.url, "_blank", "noopener");
      setA({ provider, email, fase: r.user_code ? "codice" : "incolla", codice: r.user_code, url: r.url, urlPkce: r.url_pkce, state: r.state ?? "", esito: "", inCorso: false });
    } catch (e) {
      setA({ provider, email, fase: "avvio", state: "", esito: messaggio(e), inCorso: false });
    }
  };

  // il codice Microsoft: si chiede ogni 4 s se l'accesso è fatto
  const inAttesa = a?.fase === "codice" ? a.state : "";
  useEffect(() => {
    if (!inAttesa) return;
    let finito = false;
    const timer = setInterval(() => {
      void (async () => {
        try {
          const r = await api("controlla_accesso_casella", inAttesa);
          if (finito) return;
          if (r.ok && r.stato === "completato" && r.account) {
            finito = true;
            setA(null);
            await fatto.current(r.account);
          } else if (!r.ok) {
            finito = true;
            setA((s) => s && { ...s, fase: "avvio", esito: r.error ?? "Accesso non riuscito." });
          }
        } catch {
          // riprova al giro dopo
        }
      })();
    }, 4000);
    return () => {
      finito = true;
      clearInterval(timer);
    };
  }, [inAttesa]);

  const linkDiretto = () => {
    if (!a?.urlPkce) return;
    window.open(a.urlPkce, "_blank", "noopener");
    setA({ ...a, fase: "incolla", url: a.urlPkce, esito: "" });
  };
  const conferma = async (testo: string) => {
    if (!a) return;
    const t = testo.trim();
    if (!t) return setA({ ...a, esito: "Incolla l'indirizzo finale." });
    setA({ ...a, inCorso: true, esito: "Verifico l'accesso…" });
    try {
      const r = await api("completa_accesso_casella", { provider: a.provider, codice: t, state: a.state, email: a.email });
      if (r.ok && r.account) {
        setA(null);
        await fatto.current(r.account);
      } else setA((s) => s && { ...s, inCorso: false, esito: r.error ?? "Accesso non riuscito." });
    } catch (e) {
      setA((s) => s && { ...s, inCorso: false, esito: messaggio(e) });
    }
  };
  return { a, avvia, linkDiretto, conferma, annulla: () => setA(null) };
}
type Accesso = ReturnType<typeof useAccesso>;

function Passo({ n, children }: { n: number; children: ReactNode }) {
  return (
    <div className="flex gap-3">
      <span className="grid size-6 flex-none place-items-center rounded-full bg-accent text-xs font-bold text-accent-foreground">{n}</span>
      <div className="flex min-w-0 flex-1 flex-col items-start gap-2 pt-0.5">{children}</div>
    </div>
  );
}

function VistaAccesso({ acc }: { acc: Accesso }) {
  const [testo, setTesto] = useState("");
  const a = acc.a;
  if (!a) return null;
  const nome = a.provider === "microsoft" ? "Microsoft" : "Google";
  return (
    <div id="cfg-accesso" className="mt-3 flex flex-col gap-4 rounded-lg border bg-card p-4">
      {a.fase === "avvio" && a.inCorso && (
        <p className="flex items-center gap-2 text-muted-foreground">
          <Spinner /> Avvio l&apos;accesso {nome}…
        </p>
      )}
      {a.fase === "avvio" && !a.inCorso && (
        <Button variant="outline" size="sm" onClick={() => void acc.avvia(a.provider, a.email)}>
          Riprova
        </Button>
      )}
      {a.fase === "codice" && (
        <>
          <Passo n={1}>
            <span>Il codice, già copiato:</span>
            <span className="flex items-center gap-2">
              <code id="cfg-oauth-codice" className="rounded-md bg-muted px-3 py-1 font-mono text-xl font-bold tracking-widest">
                {a.codice}
              </code>
              <Button variant="outline" size="sm" onClick={() => void navigator.clipboard?.writeText(a.codice ?? "").catch(() => {})}>
                Copia
              </Button>
            </span>
          </Passo>
          <Passo n={2}>
            <span>Incollalo nella pagina Microsoft e accedi.</span>
            <Button size="sm" variant="outline" onClick={() => window.open(a.url, "_blank", "noopener")}>
              <IconaMicrosoft className="size-3.5" /> Apri la pagina Microsoft
            </Button>
          </Passo>
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <Spinner className="size-3.5" /> In attesa dell&apos;accesso: si completa da solo.
          </p>
          {a.urlPkce && (
            <button type="button" className="self-start text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" onClick={acc.linkDiretto}>
              Il codice non va? Accedi con il link diretto
            </button>
          )}
        </>
      )}
      {a.fase === "incolla" && (
        <>
          <Passo n={1}>
            <span>Accedi nella pagina {nome}.</span>
            <Button size="sm" variant="outline" onClick={() => window.open(a.url, "_blank", "noopener")}>
              <IconaFornitore provider={a.provider} className="size-3.5" /> Apri la pagina {nome}
            </Button>
          </Passo>
          <Passo n={2}>
            <span>
              Alla fine il browser mostra un indirizzo che non si apre (
              <code className="text-xs">{a.provider === "google" ? "http://localhost/?code=…" : "msauth://…"}</code>): copialo dalla barra e incollalo qui.
            </span>
            <span className="flex w-full flex-wrap items-center gap-2">
              <Input
                id="cfg-oauth-redirect"
                className="h-8 min-w-[220px] flex-1 text-xs"
                value={testo}
                placeholder={a.provider === "google" ? "http://localhost/?code=..." : "msauth://...?code=..."}
                onChange={(e) => setTesto(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void acc.conferma(testo)}
              />
              <Button id="btn-oauth-conferma" size="sm" disabled={a.inCorso} onClick={() => void acc.conferma(testo)}>
                Conferma
              </Button>
            </span>
          </Passo>
        </>
      )}
      {a.esito && (
        <p id="cfg-oauth-esito" role="status" className="text-xs text-muted-foreground">
          {a.esito}
        </p>
      )}
      <Button variant="ghost" size="sm" className="self-start" onClick={acc.annulla}>
        Annulla
      </Button>
    </div>
  );
}

// ---- Una casella ---------------------------------------------------------------------------------------------------
function PannelloCasella({
  casella,
  nome,
  colore,
  uniti,
  appenaEntrato,
  acc,
  loghi,
  onLoghiCambiati,
  onRimossa,
  onApri,
}: {
  casella: CasellaAccesso;
  nome: string;
  colore: string;
  uniti: string[];
  appenaEntrato: boolean;
  acc: Accesso;
  loghi: Record<string, VoceLogo>;
  onLoghiCambiati: () => Promise<void>;
  onRimossa: () => Promise<void>;
  /** Apre la pagina di un calendario inoltrato qui. */
  onApri: (k: string) => void;
}) {
  const oe = useOe();
  const cal = oe.calendari.find((c) => c.id === idCasella(casella.account));
  const inoltrate = calendariInoltrati(casella, oe.calendari);
  const [valore, setValore] = useState(nome);
  useEffect(() => setValore(nome), [nome]);
  const [conferma, setConferma] = useState(false);
  const [rimuovendo, setRimuovendo] = useState(false);
  const [errore, setErrore] = useState("");
  const accessoQui = acc.a && acc.a.email.toLowerCase() === casella.account.toLowerCase();
  const stato = statoDi(casella);
  const fornitore = casella.provider === "google" ? "Google" : "Microsoft";

  const rinomina = async () => {
    if (valore.trim() === nome) return;
    await api("set_account_alias", casella.account, valore.trim());
    await oe.aggiornaCalendari();
    oe.ricarica();
    // il server sposta anche logo, gruppi e "secondario" sul nome nuovo
    await onLoghiCambiati();
  };
  const rimuovi = async () => {
    setRimuovendo(true);
    setErrore("");
    try {
      const r = await api("rimuovi_casella", casella.account);
      if (!r.ok) throw new Error(r.error ?? "Casella non rimossa");
      await onRimossa();
    } catch (e) {
      setErrore(messaggio(e));
      setRimuovendo(false);
    }
  };

  return (
    <div data-pannello-casella={casella.account}>
      <div className="flex items-center gap-4">
        <AvatarLogo nome={nome} colore={colore} lato={60} segno voce={loghi[nome]} onCambiato={onLoghiCambiati} />
        <div className="min-w-0 flex-1">
          <h2 className="flex flex-wrap items-center gap-2 text-lg font-semibold">
            {nome}
            <span className="flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
              <IconaFornitore provider={casella.provider} className="size-3" />
              {fornitore}
            </span>
          </h2>
          <div className="truncate text-xs text-muted-foreground">{casella.account}</div>
          <div id="casella-stato" className="mt-1 flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className={cn("size-2 rounded-full", PALLINO[stato])} />
            {casella.scaduto ? "Accesso scaduto" : stato === "errore" ? "Lettura non riuscita" : stato === "avviso" ? "Da controllare" : "Connesso"}
            {casella.msg && !casella.scaduto && <span>· {casella.msg}</span>}
          </div>
        </div>
      </div>

      {appenaEntrato && (
        <p role="status" className="mt-3 rounded-md border-l-[3px] border-emerald-600 bg-emerald-600/10 px-3 py-2 text-[13px]">
          Accesso fatto: il calendario arriva in pagina alla prossima lettura.
        </p>
      )}
      {casella.scaduto && (
        <div id="casella-scaduta" className="mt-3 rounded-lg border border-red-600/30 bg-red-600/5 p-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <b className="text-red-700 dark:text-red-400">Accesso scaduto</b>
              <p className="text-xs text-muted-foreground">{fornitore} non accetta più l&apos;accesso salvato: il calendario non si aggiorna.</p>
            </div>
            {!accessoQui && (
              <Button id="btn-accedi-di-nuovo" size="sm" variant="outline" className="bg-background" onClick={() => void acc.avvia(casella.provider, casella.account)}>
                <IconaFornitore provider={casella.provider} className="size-3.5" /> Accedi di nuovo
              </Button>
            )}
          </div>
          {accessoQui && <VistaAccesso acc={acc} />}
        </div>
      )}
      {!casella.scaduto && stato !== "ok" && casella.msg && (
        <p className="mt-3 rounded-md border-l-[3px] border-amber-500 bg-amber-500/10 px-3 py-2 text-[13px]">{casella.msg}</p>
      )}

      <div className="mt-5 flex flex-col gap-4">
        <Riga etichetta="Nome mostrato">
          <Input
            className="alias-input h-8 max-w-[260px] font-semibold"
            value={valore}
            data-account={casella.account}
            spellCheck={false}
            onChange={(e) => setValore(e.target.value)}
            onBlur={() => void rinomina()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
          <p className="mt-1 text-xs text-muted-foreground">
            {uniti.length ? `Unito con ${uniti.join(", ")}: nella barra sono una voce sola.` : "Lo stesso nome su due account li unisce in una voce sola."}
          </p>
        </Riga>
        <Riga etichetta="Su questo dispositivo">
          <MostraQui c={cal} />
        </Riga>
        {casella.provider === "microsoft" && casella.configurato && (
          <Riga etichetta="Account condiviso">
            <CondivisioneMicrosoft account={casella.account} />
          </Riga>
        )}
        <OpzioniNome nome={nome} />
        {inoltrate.length > 0 && (
          <Riga etichetta="Inoltrate qui">
            <Pillole id="casella-inoltrate" calendari={inoltrate} onApri={onApri} />
          </Riga>
        )}
      </div>

      <div className="mt-8 border-t pt-4">
        {!conferma ? (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <b>Rimuovi account</b>
              <p className="text-xs text-muted-foreground">Smette di leggere questa casella e cancella l&apos;accesso salvato. Si riaggiunge da Aggiungi account.</p>
            </div>
            <Button id="btn-rimuovi-account" variant="outline" size="sm" className="text-destructive" onClick={() => setConferma(true)}>
              Rimuovi…
            </Button>
          </div>
        ) : (
          <div role="alertdialog" aria-labelledby="rimuovi-domanda" className="rounded-lg border border-destructive/40 bg-destructive/5 p-3">
            <p id="rimuovi-domanda">
              Rimuovere <b>{casella.account}</b>? Il suo calendario sparisce dalla pagina
              {inoltrate.length ? `; ${inoltrate.map((c) => c.alias || c.account).join(", ")} restano senza riunioni` : ""}.
            </p>
            <div className="mt-3 flex gap-2">
              <Button id="btn-rimuovi-conferma" variant="destructive" size="sm" disabled={rimuovendo} onClick={() => void rimuovi()}>
                {rimuovendo && <Spinner className="size-3.5" />} Rimuovi
              </Button>
              <Button variant="ghost" size="sm" disabled={rimuovendo} onClick={() => setConferma(false)}>
                Annulla
              </Button>
            </div>
            {errore && <p className="mt-2 text-xs text-destructive">{errore}</p>}
          </div>
        )}
      </div>
    </div>
  );
}

/** "Altri calendari" e gruppi di un nome mostrato: valgono per tutto quello che porta quel nome. Il logo si cambia dal cerchio in cima. */
function OpzioniNome({ nome }: { nome: string }) {
  const { gruppiCalendari, salvaGruppi, secondari, impostaSecondario } = useOe();
  const nomiGruppi = Object.keys(gruppiCalendari);
  const secondario = secondari.has(nome);
  return (
    <div className="flex flex-col gap-4">
      <Riga etichetta="Nella barra">
        <label className="flex items-center gap-2 text-[13px]">
          <Switch data-secondario={nome} checked={secondario} onCheckedChange={(v) => void impostaSecondario(nome, v)} />
          In fondo, sotto &quot;Altri calendari&quot;
        </label>
      </Riga>
      <Riga etichetta="Gruppi">
        {nomiGruppi.length ? (
          <span className="flex flex-wrap gap-1" role="group" aria-label={`Gruppi di ${nome}`}>
            {nomiGruppi.map((g) => {
              const dentro = gruppiCalendari[g].includes(nome);
              return (
                <button
                  key={g}
                  type="button"
                  data-gruppo={g}
                  data-di={nome}
                  aria-pressed={dentro}
                  title={dentro ? `Togli ${nome} dal gruppo ${g}` : `Metti ${nome} nel gruppo ${g}`}
                  className={cn(
                    "rounded-full border px-2.5 py-0.5 text-xs font-medium",
                    dentro ? "border-primary bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                  )}
                  onClick={() => void salvaGruppi(impostaAppartenenza(gruppiCalendari, g, nome, !dentro))}
                >
                  {g}
                </button>
              );
            })}
          </span>
        ) : (
          <span className="pt-1 text-xs text-muted-foreground">Nessun gruppo: si creano in Gruppi.</span>
        )}
      </Riga>
    </div>
  );
}

/** I calendari sotto una voce, come pillole col logo: ognuna apre la pagina del suo calendario. Grigie quelle nascoste qui. */
function Pillole({ id, calendari, onApri }: { id: string; calendari: Calendario[]; onApri: (k: string) => void }) {
  const { pers } = useOe();
  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  return (
    <span id={id} className="flex flex-wrap gap-1.5">
      {calendari.map((c) => {
        const n = c.alias || c.account;
        return (
          <button
            key={c.id}
            type="button"
            data-apri-calendario={c.id}
            title={c.enabled ? `Apri ${n}` : `Apri ${n} (nascosto su questo dispositivo)`}
            className={cn("flex items-center gap-1.5 rounded-full border py-0.5 pl-0.5 pr-2.5 text-xs font-medium hover:bg-accent", !c.enabled && "text-muted-foreground")}
            onClick={() => onApri("calendario:" + c.id)}
          >
            <CerchioLogo nome={n} colore={c.color} src={loghi[n]} lato={20} />
            {n}
          </button>
        );
      })}
    </span>
  );
}

/** L'interruttore di un calendario su questo dispositivo, con quanti elementi ha letto. */
function MostraQui({ c }: { c?: Calendario }) {
  const { accendiCalendario } = useOe();
  if (!c) return <span className="pt-1 text-xs text-muted-foreground">Nessun calendario letto ancora.</span>;
  return (
    <label className="flex items-center gap-2 text-[13px]">
      <Switch data-mostra={c.id} checked={c.enabled} onCheckedChange={(v) => void accendiCalendario(c.id, v)} />
      Mostra nel calendario
      {c.count != null && <span className="text-xs text-muted-foreground">· {c.count} elementi</span>}
    </label>
  );
}

// ---- Un calendario sotto una casella o sotto "Da link e personali" ---------------------------------------------------------
function PannelloCalendario({
  c,
  sotto,
  loghi,
  onLoghiCambiati,
  onRimosso,
}: {
  c: Calendario;
  sotto: string;
  loghi: Record<string, VoceLogo>;
  onLoghiCambiati: () => Promise<void>;
  onRimosso: () => void;
}) {
  const oe = useOe();
  const nome = c.alias || c.account;
  const [conferma, setConferma] = useState(false);
  const [rimuovendo, setRimuovendo] = useState(false);
  const [errore, setErrore] = useState("");
  const rimuovi = async () => {
    setRimuovendo(true);
    setErrore("");
    try {
      const r = await api("remove_ics_calendar", c.id);
      if (r && r.ok === false) throw new Error(r.error ?? "Calendario non rimosso");
      await oe.aggiornaCalendari();
      oe.ricarica();
      onRimosso();
    } catch (e) {
      setErrore(messaggio(e));
      setRimuovendo(false);
    }
  };
  return (
    <div data-pannello-calendario={c.id}>
      <div className="flex items-center gap-4">
        <AvatarLogo nome={nome} colore={c.color} lato={60} segno voce={loghi[nome]} onCambiato={onLoghiCambiati} />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-lg font-semibold">{nome}</h2>
          <div className="truncate text-xs text-muted-foreground">{sotto}</div>
          {c.stato_msg && <div className="mt-1 text-xs text-muted-foreground">{c.stato_msg}</div>}
        </div>
      </div>

      <div className="mt-5 flex flex-col gap-4">
        <Riga etichetta="Su questo dispositivo">
          <MostraQui c={c} />
        </Riga>
        <OpzioniNome nome={nome} />
      </div>

      {c.is_ics && !c.builtin && (
        <div className="mt-8 border-t pt-4">
          {!conferma ? (
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="min-w-0">
                <b>Rimuovi calendario</b>
                <p className="text-xs text-muted-foreground">Smette di leggere il link. Si riaggiunge da Da link e personali.</p>
              </div>
              <Button id="btn-rimuovi-calendario" variant="outline" size="sm" className="text-destructive" onClick={() => setConferma(true)}>
                Rimuovi…
              </Button>
            </div>
          ) : (
            <div role="alertdialog" aria-labelledby="rimuovi-cal-domanda" className="rounded-lg border border-destructive/40 bg-destructive/5 p-3">
              <p id="rimuovi-cal-domanda">
                Rimuovere <b>{nome}</b>? Le sue riunioni spariscono dalla pagina.
              </p>
              <div className="mt-3 flex gap-2">
                <Button variant="destructive" size="sm" disabled={rimuovendo} onClick={() => void rimuovi()}>
                  {rimuovendo && <Spinner className="size-3.5" />} Rimuovi
                </Button>
                <Button variant="ghost" size="sm" disabled={rimuovendo} onClick={() => setConferma(false)}>
                  Annulla
                </Button>
              </div>
              {errore && <p className="mt-2 text-xs text-destructive">{errore}</p>}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---- Aggiungi account -------------------------------------------------------------------------------------------------
function PannelloAggiungi({ acc, onLink }: { acc: Accesso; onLink: () => void }) {
  const nuovo = acc.a && !acc.a.email;
  const scelte: { chiave: Provider | "link"; icona: ReactNode; titolo: string; testo: string }[] = [
    { chiave: "microsoft", icona: <IconaMicrosoft className="size-6" />, titolo: "Microsoft", testo: "Outlook, Exchange, Microsoft 365" },
    { chiave: "google", icona: <IconaGoogle className="size-6" />, titolo: "Google", testo: "Gmail, Google Workspace" },
    { chiave: "link", icona: <CalendarPlusIcon className="size-6 text-muted-foreground" />, titolo: "Calendario da link", testo: "Indirizzo iCal di Google, Outlook o altro" },
  ];
  return (
    <div>
      <Titolo sotto="Il calendario arriva in pagina appena l'accesso è fatto.">Aggiungi account</Titolo>
      {nuovo ? (
        <VistaAccesso acc={acc} />
      ) : (
        <div className="grid gap-2">
          {scelte.map((s) => (
            <button
              key={s.chiave}
              type="button"
              id={`btn-aggiungi-${s.chiave}`}
              className="flex items-center gap-3 rounded-lg border bg-card p-3 text-left hover:border-primary hover:bg-accent"
              onClick={() => (s.chiave === "link" ? onLink() : void acc.avvia(s.chiave))}
            >
              <span className="grid size-11 flex-none place-items-center rounded-lg border bg-white dark:bg-white/95">{s.icona}</span>
              <span>
                <b>{s.titolo}</b>
                <span className="block text-xs text-muted-foreground">{s.testo}</span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---- Calendari da link, festività, eventi personali -----------------------------------------------------------------
function PannelloLink({ onAggiunto, onApri }: { onAggiunto: () => void; onApri: (k: string) => void }) {
  const oe = useOe();
  const [nome, setNome] = useState("");
  const [url, setUrl] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);
  const voci = oe.calendari.filter((c) => c.is_ics || c.is_local);
  const dopo = async () => {
    await oe.aggiornaCalendari();
    oe.ricarica();
  };
  const aggiungi = async () => {
    if (!url.trim()) return setEsito("Manca il link.");
    setEsito("Verifico il link…");
    setOccupato(true);
    try {
      const r = await api("add_ics_calendar", nome.trim(), url.trim());
      if (r.ok) {
        setEsito("Aggiunto: " + r.feed?.name);
        setNome("");
        setUrl("");
        await dopo();
        onAggiunto();
      } else setEsito(r.error ?? "Non riuscito.");
    } catch (e) {
      setEsito(messaggio(e));
    } finally {
      setOccupato(false);
    }
  };
  return (
    <div>
      <Titolo sotto="Le festività, i calendari aggiunti da un link ICS e gli eventi creati qui. Ognuno ha la sua pagina, anche dalla freccia a sinistra.">
        Da link e personali
      </Titolo>
      <Pillole id="link-calendari" calendari={voci} onApri={onApri} />
      <section className="add-ics mt-6 rounded-lg border p-3">
        <h3 className="flex items-center gap-2 font-medium">
          <PlusIcon className="size-4" /> Aggiungi un calendario da link
        </h3>
        <p className="mt-1 text-xs text-muted-foreground">
          In Google Calendar: <b>Impostazioni</b>, il calendario, <b>Indirizzo segreto in formato iCal</b>.
        </p>
        <label className="mt-3 flex flex-col gap-1 text-xs font-medium">
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
          <span id="ics-result" role="status" className="text-xs text-muted-foreground">
            {esito}
          </span>
        </div>
      </section>
    </div>
  );
}

// ---- Gruppi -------------------------------------------------------------------------------------------------------------
function PannelloGruppi() {
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
    <div id="gruppi-gestione">
      <Titolo sotto="Chi sta in un gruppo si sceglie nella pagina di ogni account, alla riga Gruppi.">Gruppi</Titolo>
      <ul className="flex flex-col gap-1.5">
        {Object.keys(gruppiCalendari).map((nome) => (
          <RigaGruppo key={nome} nome={nome} />
        ))}
      </ul>
      <div className="mt-3 flex items-center gap-2">
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
    </div>
  );
}

function RigaGruppo({ nome }: { nome: string }) {
  const { gruppiCalendari, salvaGruppi } = useOe();
  const [valore, setValore] = useState(nome);
  const [errore, setErrore] = useState("");
  useEffect(() => setValore(nome), [nome]);
  const membri = gruppiCalendari[nome] ?? [];
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
      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground" title={membri.join(", ")}>
        {membri.length ? membri.join(", ") : "vuoto"}
      </span>
      <button
        type="button"
        className="flex-none rounded p-1 text-muted-foreground hover:bg-accent hover:text-destructive"
        title={`Elimina il gruppo ${nome}`}
        aria-label={`Elimina il gruppo ${nome}`}
        data-elimina-gruppo={nome}
        onClick={() => void salvaGruppi(eliminaGruppo(gruppiCalendari, nome))}
      >
        <Trash2Icon className="size-3.5" />
      </button>
      {errore && <span className="basis-full text-xs text-destructive">{errore}</span>}
    </li>
  );
}

// ---- Eliminati -----------------------------------------------------------------------------------------------------------
function PannelloEliminati() {
  const { config, ripristinaEvento } = useOe();
  const [ora] = useState(() => Date.now());
  const voci = eliminatiRecenti(config.eventi_eliminati, ora);
  return (
    <div>
      <Titolo sotto={`Si ripristinano per ${GIORNI_RIPRISTINO} giorni. Dopo restano eliminati e spariscono da qui.`}>Eliminati</Titolo>
      {voci.length ? (
        <ul id="cfg-eliminati" className="flex flex-col">
          {voci.map((v) => (
            <li key={v.chiave} className="flex items-center gap-3 border-b py-2 last:border-b-0">
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium">{v.title}</div>
                <div className="truncate text-xs text-muted-foreground">
                  {fmtRange(dataEvento(v.start, v.allDay), dataEvento(v.end || v.start, v.allDay), v.allDay)}
                  {v.alias && ` · ${v.alias}`}
                </div>
              </div>
              <span className="flex-none text-right text-xs text-muted-foreground" data-scade>
                {[eliminatoDa(v, ora), scadeTra(v, ora)].filter(Boolean).join(" · ")}
              </span>
              <Button size="sm" variant="outline" onClick={() => void ripristinaEvento(v.chiave)}>
                Ripristina
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-muted-foreground">Nessun evento eliminato negli ultimi {GIORNI_RIPRISTINO} giorni.</p>
      )}
    </div>
  );
}

// ---- Impostazioni -------------------------------------------------------------------------------------------------------
function PannelloAspetto() {
  const { tema, impostaTema, config, salvaConfig } = useOe();
  return (
    <div>
      <Titolo>Aspetto</Titolo>
      <div className="flex flex-col gap-4">
        <Riga etichetta="Tema">
          <div id="cfg-theme" className="inline-flex rounded-lg border p-0.5">
            {TEMI.map((t) => (
              <button
                key={t.valore}
                type="button"
                data-theme={t.valore}
                aria-pressed={tema === t.valore}
                className={cn("rounded-md px-3 py-1 text-[13px]", tema === t.valore ? "is-active bg-accent font-semibold text-accent-foreground" : "text-muted-foreground hover:bg-muted")}
                onClick={() => impostaTema(t.valore)}
              >
                {t.testo}
              </button>
            ))}
          </div>
        </Riga>
        <Riga etichetta="Colori">
          <label className="flex items-center gap-2 text-[13px]">
            <Switch id="cfg-colori-dinamici" checked={!!config.colori_dinamici} onCheckedChange={(v) => void salvaConfig({ colori_dinamici: v })} /> Colori dinamici
          </label>
          <p className="mt-1 text-xs text-muted-foreground">Riunioni vicine di aziende diverse mai dello stesso colore.</p>
        </Riga>
      </div>
    </div>
  );
}

/** Promemoria e Condivisione hanno già il loro titolo piccolo: qui si mostra quello del pannello. */
function PannelloIncluso({ titolo, children }: { titolo: string; children: ReactNode }) {
  return (
    <div>
      <Titolo>{titolo}</Titolo>
      <div className="[&>section>h3:first-child]:hidden">{children}</div>
    </div>
  );
}

function PannelloGenerale({ accesso2fa }: { accesso2fa: Modo2fa }) {
  const { intervalloSync, impostaIntervallo } = useOe();
  const app = useInfoApp();
  return (
    <div>
      <Titolo>Generale</Titolo>
      <div className="flex flex-col gap-4">
        <Riga etichetta="Sincronizzazione">
          <label className="flex flex-col gap-1 text-xs">
            Ricontrolla il calendario ogni
            <Select value={String(intervalloSync)} onValueChange={(v) => impostaIntervallo(Number(v))}>
              <SelectTrigger id="cfg-sync-interval" className="w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {INTERVALLI.map((i) => (
                  <SelectItem key={i.valore} value={String(i.valore)}>
                    {i.testo}
                  </SelectItem>
                ))}
                {!INTERVALLI.some((i) => i.valore === intervalloSync) && <SelectItem value={String(intervalloSync)}>{intervalloSync} secondi</SelectItem>}
              </SelectContent>
            </Select>
          </label>
        </Riga>
        {accesso2fa !== "off" && (
          <Riga etichetta="Sicurezza">
            <p className="mb-2 text-xs text-muted-foreground">Codice dell&apos;app di autenticazione{accesso2fa === "passkey" ? " e passkey" : ""}.</p>
            <a id="cfg-sicurezza" href="/sicurezza" className={buttonVariants({ variant: "outline", size: "sm" })}>
              Gestisci l&apos;accesso
            </a>
          </Riga>
        )}
        {app && (
          <Riga etichetta="Server">
            <div className="flex flex-wrap items-center gap-2">
              <b id="cfg-server">{app.host}</b>
              <a id="cfg-server-cambia" href={app.cambiaServer} className={buttonVariants({ variant: "outline", size: "sm" })}>
                Cambia server
              </a>
            </div>
          </Riga>
        )}
        <Riga etichetta="Sessione">
          <Button id="cfg-esci" variant="outline" size="sm" onClick={esci}>
            <LogOutIcon /> Esci dall&apos;account
          </Button>
        </Riga>
      </div>
    </div>
  );
}
