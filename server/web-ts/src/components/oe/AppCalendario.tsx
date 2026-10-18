"use client";

import { CalendarDaysIcon, ClockIcon, LogOutIcon, MenuIcon, MoonIcon, PanelLeftIcon, PlusIcon, SearchIcon, SettingsIcon, SmartphoneIcon, SunIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import type { Modo2fa } from "@/lib/config";
import { api } from "@/lib/oe/api";
import { vistaPerSchermo } from "@/lib/oe/logic";
import type { Evento } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { BarraComandi } from "./BarraComandi";
import { Calendario, VISTE_VALIDE, type ComandiCalendario } from "./Calendario";
import { DialogoCalendari } from "./DialogoCalendari";
import { DialogoImpostazioni } from "./DialogoImpostazioni";
import { DialogoNuovoEvento } from "./DialogoNuovoEvento";
import { Legenda } from "./Legenda";
import { MiniCalendario } from "./MiniCalendario";
import { OeProvider, useOe } from "./OeProvider";
import { PannelloLaterale } from "./PannelloLaterale";
import { SchedaEvento, type Scheda } from "./SchedaEvento";
import { Timesheet } from "./Timesheet";

/** Vero sotto i 768 px: telefono e tablet in verticale. */
function useStretto(): boolean {
  return useSyncExternalStore(
    (cb) => {
      const m = window.matchMedia("(max-width: 767.5px)");
      m.addEventListener("change", cb);
      return () => m.removeEventListener("change", cb);
    },
    () => window.matchMedia("(max-width: 767.5px)").matches,
    () => false,
  );
}

const CHIAVE_VISTA = "oe.vista-stretto";

/** La vista scelta su uno schermo stretto, che resta in questo browser: quella salvata sul server è del computer. */
function vistaLocale(): string | null {
  try {
    const v = localStorage.getItem(CHIAVE_VISTA);
    return v && (VISTE_VALIDE as readonly string[]).includes(v) ? v : null;
  } catch {
    return null;
  }
}

/** `pubblico`: la pagina senza accesso, gli stessi componenti con gli impegni ridotti a "Occupato" e niente che scriva. */
/** La pagina dello schermo del telefono (servizio schermo, dietro la stessa sessione): il telefono, già adattato alla finestra. */
const SCHERMO_TELEFONO = "/telefono/#!action=stream&udid=telefono%3A5555&player=webcodecs&fitToScreen=true";

export function AppCalendario({ pubblico, accesso2fa = "off" }: { pubblico?: { gruppo: string }; accesso2fa?: Modo2fa }) {
  return (
    <OeProvider pubblico={pubblico}>
      <Guscio accesso2fa={accesso2fa} />
    </OeProvider>
  );
}

function Guscio({ accesso2fa }: { accesso2fa: Modo2fa }) {
  const oe = useOe();
  const stretto = useStretto();
  const comandi = useRef<ComandiCalendario>(null);
  const [sezione, setSezione] = useState<"calendario" | "timesheet">("calendario");
  const [pannello, setPannello] = useState(true);
  const [menu, setMenu] = useState(false);
  const [railLarga, setRailLarga] = useState(false);
  const [dialogo, setDialogo] = useState<null | "calendari" | "evento" | "impostazioni">(null);
  const [scheda, setScheda] = useState<Scheda | null>(null);
  const [titolo, setTitolo] = useState("");
  const [campoRicerca, setCampoRicerca] = useState("");

  // la ricerca filtra gli eventi già in pagina: nessuna nuova interrogazione, la lista si stringe mentre si scrive
  useEffect(() => {
    const t = setTimeout(() => oe.impostaRicerca(campoRicerca), 160);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [campoRicerca]);

  const vista = oe.periodo?.tipo ?? "";
  const vistaIniziale = (() => {
    const locale = stretto ? vistaLocale() : null;
    if (locale) return locale;
    const salvata = oe.vistaIniziale;
    const v = salvata && (VISTE_VALIDE as readonly string[]).includes(salvata) ? salvata : "timeGridWeek";
    return vistaPerSchermo(v, stretto);
  })();

  const cambiaVista = (v: string) => {
    comandi.current?.cambiaVista(v);
    // la vista scelta si ricorda: sul server quella del computer, in questo browser quella di uno schermo stretto
    if (!stretto) void oe.salvaConfig({ vista: v });
    else {
      try {
        localStorage.setItem(CHIAVE_VISTA, v);
      } catch {
        // vale in questa pagina e non resta
      }
    }
  };
  const onDatesSet = useCallback(
    (i: { start: Date; end: Date; tipo: string; titolo: string }) => {
      setTitolo(i.titolo);
      oe.impostaPeriodo({ start: i.start, end: i.end, tipo: i.tipo });
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [oe.impostaPeriodo],
  );
  const onEventClick = useCallback(
    (evento: Evento, titolo: string, inizio: Date | null, fine: Date | null, x: number, y: number) => {
      if (!oe.pubblico) setScheda({ evento, titolo, inizio, fine, x, y }); // pubblica: un impegno non ha altro da dire
    },
    [oe.pubblico],
  );
  const chiudiScheda = useCallback(() => setScheda(null), []);

  const elimina = async (uid: string) => {
    await api("delete_local_event", uid);
    setScheda(null);
    await oe.aggiornaCalendari();
    oe.ricarica();
  };
  const esci = () => {
    fetch("/api/auth/sign-out", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" })
      .catch(() => {})
      .finally(() => {
        window.location.href = "/login";
      });
  };
  const vaiA = (d: Date) => {
    setSezione("calendario");
    setMenu(false);
    comandi.current?.vaiA(d);
  };

  const senzaTeams = new Set(oe.pers.interfaccia.calendari_senza_teams ?? []);
  const pub = oe.pubblico;
  const contenutoPannello = (
    <div className="flex flex-col gap-4">
      {!pub && (
        <div className="px-3">
          <Button id="btn-new-event" className="w-full" onClick={() => { setMenu(false); setDialogo("evento"); }}>
            <PlusIcon /> Nuovo evento
          </Button>
        </div>
      )}
      <MiniCalendario onVaiA={vaiA} />
      {!pub && <Legenda onGestisci={() => { setMenu(false); setDialogo("calendari"); }} />}
    </div>
  );

  const nessunRisultato = oe.ricerca.trim() && oe.eventiGriglia.length === 0;
  const voci = (
    <>
      <VoceRail attiva={sezione === "calendario"} id="nav-calendar" etichetta="Calendario" larga={railLarga || stretto} onClick={() => { setSezione("calendario"); setMenu(false); }}>
        <CalendarDaysIcon />
      </VoceRail>
      {!pub && (
        <VoceRail attiva={sezione === "timesheet"} id="nav-timesheet" etichetta="Timesheet" larga={railLarga || stretto} onClick={() => { setSezione("timesheet"); setMenu(false); }}>
          <ClockIcon />
        </VoceRail>
      )}
      {!pub && (
        <VoceRail id="nav-phone" etichetta="Telefono" titolo="Schermo del telefono" larga={railLarga || stretto} onClick={() => { setMenu(false); window.open(SCHERMO_TELEFONO, "_blank", "noopener"); }}>
          <SmartphoneIcon />
        </VoceRail>
      )}
      <div className="flex-1" />
      <VoceRail id="nav-theme" etichetta="Tema" larga={railLarga || stretto} onClick={() => oe.impostaTema(oe.temaScuro ? "light" : "dark")} titolo="Cambia tema">
        {/* le due icone, scelte dalla classe del tema: il tema salvato si conosce solo nel browser, e un'icona scelta in JavaScript non coinciderebbe con quella del server */}
        <SunIcon className="hidden dark:block" />
        <MoonIcon className="dark:hidden" />
      </VoceRail>
      {!pub && (
        <>
          <VoceRail id="nav-settings" etichetta="Impostazioni" larga={railLarga || stretto} onClick={() => { setMenu(false); setDialogo("impostazioni"); }}>
            <SettingsIcon />
          </VoceRail>
          <VoceRail id="nav-logout" etichetta="Esci" larga={railLarga || stretto} onClick={esci}>
            <LogOutIcon />
          </VoceRail>
        </>
      )}
    </>
  );

  return (
    <div className="app flex h-dvh flex-col bg-background text-foreground">
      <header className="topbar flex h-12 flex-none items-center gap-2 border-b bg-card px-2 sm:px-3.5">
        <Button variant="ghost" size="icon" className="md:hidden" aria-label="Menu" id="menu-btn" onClick={() => setMenu(true)}>
          <MenuIcon />
        </Button>
        <span className="topbar-title flex flex-none items-center gap-2 text-xl font-bold tracking-tight">
          <img src="/icon.png" alt="" width={28} height={28} className="size-7 flex-none" />
          {pub ? (
            <span className="truncate">
              Disponibilità
              {oe.gruppoPubblico && <span className="bg-gradient-to-br from-primary to-[#8b8ff0] bg-clip-text text-transparent">{" · "}{oe.gruppoPubblico}</span>}
            </span>
          ) : (
            <span>
              OE<span className="ml-1 bg-gradient-to-br from-primary to-[#8b8ff0] bg-clip-text text-transparent">Copilot</span>
            </span>
          )}
        </span>
        <div className={cn("mx-auto flex w-full min-w-0 max-w-[720px] flex-1 items-center", pub && "invisible")}>
          <div id="search-wrap" className="search relative flex h-9 w-full min-w-0 items-center gap-2.5 rounded-lg bg-muted px-3 text-muted-foreground focus-within:bg-card focus-within:ring-2 focus-within:ring-primary">
            <SearchIcon className="size-4 flex-none" />
            <input
              id="search"
              type="search"
              value={campoRicerca}
              onChange={(e) => setCampoRicerca(e.target.value)}
              placeholder="Cerca tra le riunioni"
              autoComplete="off"
              className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
            />
            {campoRicerca && (
              <button id="search-clear" className="search-clear rounded p-1 hover:bg-accent" aria-label="Cancella ricerca" onClick={() => { setCampoRicerca(""); oe.impostaRicerca(""); }}>
                <XIcon className="size-3.5" />
              </button>
            )}
          </div>
        </div>
      </header>

      <div className="body flex min-h-0 flex-1">
        <nav className={cn("rail hidden flex-none flex-col gap-1 border-r bg-sidebar p-1.5 transition-[width] md:flex", railLarga ? "w-44" : "w-12")}>
          <button id="rail-expand" className="flex h-9 flex-none items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground" title="Espandi o comprimi il menu" aria-label="Espandi menu" onClick={() => setRailLarga(!railLarga)}>
            <PanelLeftIcon className="size-5" />
          </button>
          <div className="mx-1 my-0.5 h-px bg-border" />
          {voci}
        </nav>

        {sezione === "calendario" && pannello && (
          <PannelloLaterale>
            <h2 className="sidepane-title mb-3 px-4 text-lg font-bold">Calendario</h2>
            {contenutoPannello}
          </PannelloLaterale>
        )}

        <main className="main flex min-w-0 flex-1 flex-col gap-2 p-2">
          {sezione === "calendario" && (
            <BarraComandi
              titolo={titolo}
              vista={vista || vistaIniziale}
              stretto={stretto}
              onPannello={() => (window.matchMedia("(min-width: 1024px)").matches ? setPannello(!pannello) : setMenu(true))}
              onOggi={() => comandi.current?.oggi()}
              onPrima={() => comandi.current?.prima()}
              onDopo={() => comandi.current?.dopo()}
              onVista={cambiaVista}
            />
          )}
          {oe.sincronizzando && (
            <div id="sync-bar" className="sync-bar h-[3px] flex-none overflow-hidden rounded bg-accent">
              <span className="block h-full w-2/5 animate-[scorre_1.1s_ease-in-out_infinite] rounded bg-primary" />
            </div>
          )}
          {oe.datiVecchi && (
            <div id="stale" className="stale flex-none rounded-md border border-dashed bg-muted px-3 py-1.5 text-[12.5px] text-muted-foreground">
              {oe.datiVecchi}
            </div>
          )}
          {(oe.avviso || nessunRisultato) && (
            <div
              id="banner"
              className={cn("banner flex-none rounded-md border-l-[3px] px-3 py-2 text-[13px]", oe.avviso?.tipo === "errore" ? "banner-errore border-red-600 bg-red-600/10 font-semibold" : "border-primary bg-accent")}
            >
              {oe.avviso ? oe.avviso.testo : `Nessun risultato per “${oe.ricerca.trim()}”.`}
            </div>
          )}

          <section id="view-calendar" className={cn("surface min-h-0 flex-1 overflow-hidden rounded-lg border bg-card p-1.5 shadow-sm", sezione !== "calendario" && "hidden")}>
            <div id="calendar" className="h-full">
              {oe.pronta ? (
                <Calendario ref={comandi} vistaIniziale={vistaIniziale} onDatesSet={onDatesSet} onEventClick={onEventClick} />
              ) : (
                <div className="grid h-full place-items-center text-sm text-muted-foreground">Caricamento…</div>
              )}
            </div>
          </section>
          {sezione === "timesheet" && <Timesheet />}
        </main>
      </div>

      {!pub && <SchedaEvento scheda={scheda} senzaTeams={senzaTeams} onChiudi={chiudiScheda} onElimina={(uid) => void elimina(uid)} />}

      <Sheet open={menu} onOpenChange={setMenu}>
        <SheetContent side="left" className="w-[19rem] overflow-y-auto p-0">
          <SheetHeader className="border-b p-4">
            <SheetTitle>OE Copilot</SheetTitle>
            <SheetDescription className="sr-only">Menu e calendari</SheetDescription>
          </SheetHeader>
          <div className="flex flex-col gap-4 py-3">
            <div className="flex flex-col gap-1 px-2 md:hidden">{voci}</div>
            {sezione === "calendario" && <div className="border-t pt-3">{contenutoPannello}</div>}
          </div>
        </SheetContent>
      </Sheet>

      {!pub && (
        <>
          <DialogoCalendari aperto={dialogo === "calendari"} onChiudi={() => setDialogo(null)} />
          <DialogoNuovoEvento aperto={dialogo === "evento"} onChiudi={() => setDialogo(null)} />
          <DialogoImpostazioni aperto={dialogo === "impostazioni"} onChiudi={() => setDialogo(null)} accesso2fa={accesso2fa} />
        </>
      )}
    </div>
  );
}

function VoceRail({ id, etichetta, titolo, attiva, larga, onClick, children }: { id: string; etichetta: string; titolo?: string; attiva?: boolean; larga: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      id={id}
      title={titolo ?? etichetta}
      onClick={onClick}
      aria-current={attiva ? "page" : undefined}
      className={cn(
        "rail-btn relative flex h-10 flex-none items-center gap-3 rounded-lg px-[9px] text-[13.5px] hover:bg-accent hover:text-foreground [&_svg]:size-[22px]",
        attiva ? "is-active bg-accent font-semibold text-primary" : "text-muted-foreground",
        !larga && "justify-center px-0 md:w-full",
      )}
    >
      {children}
      {larga && <span className="rail-label">{etichetta}</span>}
    </button>
  );
}
