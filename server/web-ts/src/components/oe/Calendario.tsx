"use client";

import itLocale from "@fullcalendar/core/locales/it";
import type { CalendarApi, DateSelectArg, DateSpanApi, DatesSetArg, DayHeaderContentArg, EventClickArg, EventContentArg, CalendarOptions } from "@fullcalendar/core";
import dayGridPlugin from "@fullcalendar/daygrid";
import interactionPlugin from "@fullcalendar/interaction";
import listPlugin from "@fullcalendar/list";
import timeGridPlugin from "@fullcalendar/timegrid";
import FullCalendar from "@fullcalendar/react";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { VISTA_3G, chiaveGiorno, dataEvento, inAlto, iniziali, mappaLoghi, orariInCima, spezzaAl, spostaGiorni } from "@/lib/oe/logic";
import type { Evento } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import "./calendario.css";
import { ImgLogo } from "./ImgLogo";
import { useOe } from "./OeProvider";
import { useScorrimento, type Scorrimento } from "./useScorrimento";

export { VISTA_3G };
export const VISTE_VALIDE = ["timeGridDay", VISTA_3G, "timeGridWeek", "dayGridMonth", "listWeek"] as const;
export type Vista = (typeof VISTE_VALIDE)[number];

/** Le viste che scorrono sotto il dito (useScorrimento): giorni visibili, giorni disegnati in più per lato, giorni di un passo. */
const SCORRE: Record<string, Scorrimento & { titolo: Intl.DateTimeFormatOptions }> = {
  timeGridDay: { visibili: 1, bordo: 2, passo: 1, titolo: { day: "numeric", month: "long", year: "numeric" } },
  [VISTA_3G]: { visibili: 3, bordo: 3, passo: 1, titolo: { day: "numeric", month: "long", year: "numeric" } },
  timeGridWeek: { visibili: 7, bordo: 7, passo: 7, titolo: { day: "numeric", month: "short", year: "numeric" } },
};

/** FullCalendar dà alla durata delle sue viste Giorno e Settimana la precedenza su visibleRange: qui sono viste nostre con un altro
 * nome. Fuori da questo file (configurazione salvata, barra, pagina) restano timeGridDay e timeGridWeek. */
const A_FC: Record<string, string> = { timeGridDay: "timeGridGiorno", timeGridWeek: "timeGridSettimana" };
const DA_FC: Record<string, string> = Object.fromEntries(Object.entries(A_FC).map(([a, b]) => [b, a]));
const aFc = (v: string) => A_FC[v] ?? v;
const daFc = (v: string) => DA_FC[v] ?? v;

/** Il periodo che FullCalendar disegna per una data: quello visibile (la settimana parte dal lunedì) con i giorni nascosti ai lati. */
const intervallo = (s: Scorrimento) => (d: Date) => {
  const inizio = s.passo === 7 ? spostaGiorni(d, -((d.getDay() + 6) % 7)) : spostaGiorni(d, 0);
  return { start: spostaGiorni(inizio, -s.bordo), end: spostaGiorni(inizio, s.visibili + s.bordo) };
};

/** Le viste con il nome di FullCalendar: giorno, tre giorni e settimana scorrono sotto il dito come in Outlook, FullCalendar disegna anche i giorni accanto. */
export const VISTE_FC: NonNullable<CalendarOptions["views"]> = {
  // in cima (inAlto) al più tre righe, poi "+N"
  [A_FC.timeGridDay]: { type: "timeGrid", visibleRange: intervallo(SCORRE.timeGridDay), dateIncrement: { days: 1 }, dayMaxEventRows: 3 },
  [VISTA_3G]: { type: "timeGrid", visibleRange: intervallo(SCORRE[VISTA_3G]), dateIncrement: { days: 1 }, dayMaxEventRows: 3 },
  [A_FC.timeGridWeek]: { type: "timeGrid", visibleRange: intervallo(SCORRE.timeGridWeek), dateIncrement: { weeks: 1 }, dayMaxEventRows: 3 },
  dayGridMonth: { dayHeaderFormat: { weekday: "short" } },
};

/** Un periodo avanti (1) o indietro (-1), le frecce della barra. prev() e next() di FullCalendar mettono la data corrente al primo giorno del
 * periodo, che nelle viste che scorrono è uno dei giorni nascosti a sinistra: la freccia dopo partiva da lì (avanti una volta, poi indietro), e
 * così il dito, lo zoom e il cambio di vista. Qui la data si sposta di un passo come col dito e con le frecce della tastiera. */
export function unPasso(a: CalendarApi, verso: 1 | -1) {
  const s = SCORRE[daFc(a.view.type)];
  if (s) a.incrementDate({ days: verso * s.passo });
  else if (verso > 0) a.next();
  else a.prev();
}

export type ComandiCalendario = {
  oggi: () => void;
  prima: () => void;
  dopo: () => void;
  vaiA: (d: Date) => void;
  cambiaVista: (v: string) => void;
  /** Toglie lo spazio scelto sulla griglia (la creazione rapida si è chiusa). */
  deseleziona: () => void;
  /** Sposta lo spazio scelto dove dicono i campi della creazione rapida. */
  seleziona: (inizio: Date, fine: Date, tuttoIlGiorno: boolean) => void;
};

/** Uno spazio scelto sulla griglia, per un evento nuovo: x e y sono il punto del clic. */
export type Selezione = { inizio: Date; fine: Date; tuttoIlGiorno: boolean; x: number; y: number };

type Props = {
  vistaIniziale: string;
  onDatesSet: (info: { start: Date; end: Date; tipo: string; titolo: string }) => void;
  onEventClick: (ev: Evento, titolo: string, inizio: Date | null, fine: Date | null, x: number, y: number) => void;
  /** Clic (o trascinamento in verticale) su uno spazio libero: come in Outlook, apre la creazione di un evento lì. */
  onSeleziona?: (s: Selezione) => void;
};

/** Uno spazio si sceglie dentro un giorno solo: il trascinamento in orizzontale è lo scorrimento dei giorni (useScorrimento). */
const dentroUnGiorno = (s: DateSpanApi) => s.allDay || chiaveGiorno(s.start) === chiaveGiorno(new Date(s.end.getTime() - 1));

/** Il riquadro da mezz'ora sotto il mouse, come in Outlook: la cornice della colonna del giorno (dove FullCalendar mette sfondo, selezione e
 * riunioni) e l'altezza della riga dell'orario, misurata dalla cornice. Null fuori dalla griglia o sopra una riunione. */
function cellaSotto(x: number, y: number): { cornice: HTMLElement; top: number; height: number } | null {
  const sotto = document.elementsFromPoint(x, y);
  if (sotto.some((e) => e.closest(".fc-event"))) return null;
  const cornice = sotto.find((e) => e.matches(".fc-timegrid-col[data-date]"))?.querySelector<HTMLElement>(".fc-timegrid-col-frame");
  const riga = sotto.find((e) => e.matches(".fc-timegrid-slot-lane[data-time]"));
  if (!cornice || !riga) return null;
  const r = riga.getBoundingClientRect();
  return { cornice, top: r.top - cornice.getBoundingClientRect().top, height: r.height };
}

/** Orario della riga in cima alla parte visibile della griglia, per rimettersi lì dopo un ridisegno. */
function oraInCima(radice: HTMLElement | null): string | null {
  const sc = radice?.querySelector(".fc-scroller-liquid-absolute");
  if (!sc || !radice) return null;
  const y = sc.getBoundingClientRect().top;
  let migliore: string | null = null;
  let distanza = Infinity;
  radice.querySelectorAll<HTMLElement>(".fc-timegrid-slot-lane[data-time]").forEach((el) => {
    const d = Math.abs(el.getBoundingClientRect().top - y);
    if (d < distanza) {
      distanza = d;
      migliore = el.dataset.time ?? null;
    }
  });
  return migliore;
}

export const Calendario = forwardRef<ComandiCalendario, Props>(function Calendario({ vistaIniziale, onDatesSet, onEventClick, onSeleziona }, ref) {
  const { eventiGriglia, infoGiorni, pers, zoom, periodo, pubblico, config } = useOe();
  // senza riga in cima: la pagina pubblica, e chi ha spento "Eventi di tutto il giorno" (a quel punto in cima non c'è più niente da mettere:
  // le riunioni lunghe tornano nella griglia e la riga si toglie, che lo spazio serve alle ore)
  const senzaRiga = pubblico || config.nascondi_tutto_il_giorno === true;
  const fc = useRef<FullCalendar>(null);
  const radice = useRef<HTMLDivElement>(null);
  const [tipo, setTipo] = useState<string>(vistaIniziale);
  const tipoCorrente = useRef<string>(vistaIniziale);
  const api = () => fc.current?.getApi();

  useImperativeHandle(ref, () => ({
    oggi: () => api()?.today(),
    prima: () => {
      const a = api();
      if (a) unPasso(a, -1);
    },
    dopo: () => {
      const a = api();
      if (a) unPasso(a, 1);
    },
    vaiA: (d) => api()?.gotoDate(d),
    cambiaVista: (v) => api()?.changeView(aFc(v)),
    deseleziona: () => api()?.unselect(),
    seleziona: (inizio, fine, tuttoIlGiorno) => api()?.select({ start: inizio, end: fine, allDay: tuttoIlGiorno }),
  }));

  // Il riquadro sotto il mouse nelle viste a colonne: un elemento solo, spostato a ogni movimento senza ridisegnare il calendario. Sta
  // nella cornice della colonna, sopra lo sfondo (z-index 2, dopo di lui) e sotto le riunioni (3): una riunione accanto resta sopra, e la
  // colonna lo taglia dove scorre. Lo crea l'effetto e non React, perché finisce dentro il DOM di FullCalendar; se FullCalendar ridisegna la
  // colonna, il movimento dopo lo rimette. Non col dito (niente passaggio) e non mentre si trascina o si scorre.
  const crea = !pubblico && !!onSeleziona;
  useEffect(() => {
    const el = radice.current;
    if (!el || !crea || !tipo.startsWith("timeGrid")) return;
    const box = document.createElement("div");
    box.className = "cella-sotto";
    box.setAttribute("aria-hidden", "true");
    const nascondi = () => (box.style.display = "none");
    const muovi = (e: PointerEvent) => {
      const r = e.pointerType === "mouse" && !e.buttons && !el.dataset.trascina ? cellaSotto(e.clientX, e.clientY) : null;
      if (!r) return nascondi();
      if (box.parentElement !== r.cornice) r.cornice.appendChild(box);
      Object.assign(box.style, { display: "block", top: `${r.top + 1}px`, height: `${r.height - 1}px` });
    };
    el.addEventListener("pointermove", muovi);
    el.addEventListener("pointerleave", nascondi);
    el.addEventListener("pointerdown", nascondi);
    el.addEventListener("scroll", nascondi, true);
    el.addEventListener("wheel", nascondi, { passive: true });
    return () => {
      box.remove();
      el.removeEventListener("pointermove", muovi);
      el.removeEventListener("pointerleave", nascondi);
      el.removeEventListener("pointerdown", nascondi);
      el.removeEventListener("scroll", nascondi, true);
      el.removeEventListener("wheel", nascondi);
    };
  }, [crea, tipo]);

  // dove è partito il clic: uno spostamento soprattutto in orizzontale è lo scorrimento dei giorni, non uno spazio scelto
  const partenza = useRef<{ x: number; y: number } | null>(null);
  const select = useCallback(
    (s: DateSelectArg) => {
      const e = s.jsEvent;
      // senza evento del mouse è select() chiamata da qui (seleziona: i campi della creazione rapida), che sa già dove si trova
      if (!e) return;
      const p = partenza.current;
      if (!onSeleziona || (p &&Math.abs(e.clientX - p.x) >= 8 && Math.abs(e.clientX - p.x) > Math.abs(e.clientY - p.y))) {
        s.view.calendar.unselect();
        return;
      }
      onSeleziona({ inizio: s.start, fine: s.end, tuttoIlGiorno: s.allDay, x: e.clientX, y: e.clientY });
    },
    [onSeleziona],
  );

  useScorrimento(radice, () => {
    const s = SCORRE[tipoCorrente.current];
    const a = api();
    return s && a ? { api: a, s } : null;
  });

  // Lo zoom alza o abbassa le righe: FullCalendar rimisura la griglia ma non le posizioni degli eventi,
  // quindi si ridisegna la vista rimettendosi sullo stesso orario.
  const primoZoom = useRef(true);
  useEffect(() => {
    if (primoZoom.current) {
      primoZoom.current = false;
      return;
    }
    const t = setTimeout(() => {
      const a = api();
      if (!a) return;
      const ora = oraInCima(radice.current);
      a.changeView(a.view.type);
      a.updateSize();
      if (ora) a.scrollToTime(ora);
    }, 0);
    return () => clearTimeout(t);
  }, [zoom]);

  const datesSet = useCallback(
    (i: DatesSetArg) => {
      const tipo = daFc(i.view.type);
      tipoCorrente.current = tipo;
      setTipo(tipo);
      const s = SCORRE[tipo];
      if (!s) return onDatesSet({ start: i.start, end: i.end, tipo, titolo: i.view.title });
      // i giorni nascosti ai lati non sono il periodo della pagina: titolo, eventi e minicalendario seguono quelli che si vedono
      const start = spostaGiorni(i.start, s.bordo);
      const end = spostaGiorni(i.end, -s.bordo);
      const separatore = i.view.calendar.getOption("titleRangeSeparator");
      onDatesSet({ start, end, tipo, titolo: i.view.calendar.formatRange(start, spostaGiorni(end, -1), { ...s.titolo, separator: separatore }) });
    },
    [onDatesSet],
  );

  const eventClick = useCallback(
    (a: EventClickArg) => {
      a.jsEvent.preventDefault();
      const p = a.event.extendedProps as Evento;
      // in cima l'evento può avere altri orari (inAlto, spezzaAl): la scheda mostra i suoi
      const su = a.event.allDay;
      onEventClick(p, a.event.title, su ? dataEvento(p.start, p.allDay) : a.event.start, su ? dataEvento(p.end || p.start, p.allDay) : a.event.end, a.jsEvent.clientX, a.jsEvent.clientY);
    },
    [onEventClick],
  );

  // nelle viste a colonne le riunioni lunghe vanno in cima, con il titolo nel primo giorno che si vede. Non nella pagina pubblica: lì sono
  // blocchi "Occupato" uniti, e in cima lascerebbero libere nella griglia le ore in cui si è occupati
  const primoGiorno = periodo ? chiaveGiorno(periodo.start) : "";
  const eventi = useMemo(
    () => (SCORRE[tipo] && !senzaRiga ? eventiGriglia.flatMap((e) => spezzaAl(inAlto(e), primoGiorno)) : eventiGriglia),
    [eventiGriglia, tipo, primoGiorno, senzaRiga],
  );

  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  const sigle = pers.interfaccia.sigle ?? {};

  const eventContent = useCallback(
    (arg: EventContentArg) => {
      const p = arg.event.extendedProps as Evento;
      const tipo = daFc(arg.view.type);
      if (tipo === "dayGridMonth" || tipo.startsWith("list") || arg.event.allDay) {
        // mese, agenda e la riga in cima alle colonne: una riga sola, orario e titolo
        const logoRiga = loghi[p.alias || p.account || ""];
        const orario = arg.event.allDay && !p.allDay ? orariInCima(new Date(p.start), new Date(p.end)) : arg.timeText;
        return (
          <div className="ev-line">
            {logoRiga && <ImgLogo key={logoRiga} src={logoRiga} className="ev-logo" alternativa={null} />}
            {orario && <span className="ev-time">{orario} </span>}
            {arg.event.title}
          </div>
        );
      }
      const alias = p.alias || p.account || "";
      const logo = loghi[alias];
      const sigla = sigle[alias] || iniziali(alias);
      const angolo = logo ? (
        <span className="ev-corner" title={alias}>
          {/* se il logo non si carica restano le iniziali nel colore dell'azienda */}
          <ImgLogo key={logo} src={logo} alternativa={<span className="grid size-full place-items-center" style={{ background: p.color }}>{sigla}</span>} />
        </span>
      ) : alias ? (
        <span className="ev-corner iniziali" style={{ background: p.color }} title={alias}>
          {sigla}
        </span>
      ) : null;
      // nella vista Giorno e a 3 giorni c'è la larghezza per scrivere anche di che azienda è la riunione
      const azienda = tipo === "timeGridDay" || tipo === VISTA_3G ? alias : "";
      // il logo (o le iniziali) sta dentro l'evento, a destra della riga dell'orario: sempre nello stesso punto,
      // mai a cavallo del bordo (prima usciva e finiva sopra l'evento accanto o sotto quello prima)
      return (
        <div className="ev-body">
          {(arg.timeText || angolo) && (
            <div className="ev-meta">
              <span className="ev-meta-text">
                {arg.timeText}
                {azienda && <span className="ev-az"> · {azienda}</span>}
              </span>
              {angolo}
            </div>
          )}
          <div className="ev-title">{arg.event.title}</div>
        </div>
      );
    },
    [loghi, sigle],
  );

  const festivi = infoGiorni.festivi;

  // Weekend e festività si distinguono a colpo d'occhio; le festività stanno sotto la data, gli eventi di un giorno intero e le
  // riunioni lunghe (inAlto) nella riga in cima alle colonne.
  const classiGiorno = useCallback(
    (d: Date) => {
      const dow = d.getDay();
      const out: string[] = [];
      if (dow === 0 || dow === 6) out.push("giorno-weekend");
      if (chiaveGiorno(d) in festivi) out.push("giorno-festivo");
      return out;
    },
    [festivi],
  );

  const classiIntestazione = useCallback((a: { date: Date }) => classiGiorno(a.date), [classiGiorno]);
  const classiCella = classiIntestazione;

  const dayHeaderContent = useCallback(
    (arg: DayHeaderContentArg) => {
      if (!arg.view.type.startsWith("timeGrid")) return arg.text;
      const g = chiaveGiorno(arg.date);
      return (
        <>
          <span>{arg.text}</span>
          {festivi[g] && (
            <span className="fc-day-extra">
              <span className="fc-festivo-pill" title={festivi[g]}>
                {festivi[g]}
              </span>
            </span>
          )}
        </>
      );
    },
    [festivi],
  );

  return (
    <div
      ref={radice}
      className={cn("calendario h-full", SCORRE[tipo] && "scorre", tipo === "timeGridWeek" && "vista-sett", crea && "crea")}
      style={{ ["--slot-h" as string]: `${zoom}px` }}
      onPointerDownCapture={(e) => (partenza.current = { x: e.clientX, y: e.clientY })}
    >
      <FullCalendar
        ref={fc}
        plugins={[dayGridPlugin, timeGridPlugin, listPlugin, interactionPlugin]}
        initialView={aFc(vistaIniziale)}
        views={VISTE_FC}
        locale={itLocale}
        firstDay={1}
        nowIndicator
        height="100%"
        // FullCalendar rimisura la griglia solo su un evento resize della finestra, e di norma aspetta 100 ms dall'ultimo: trascinando la colonna dei
        // calendari (PannelloLaterale) la griglia restava indietro. Il lavoro di una rimisura è circa 2 ms, quindi si fa subito
        windowResizeDelay={0}
        // altezza naturale con scorrimento: gli slot restano leggibili e una riunione da mezz'ora ha spazio
        expandRows={false}
        slotMinTime="07:00:00"
        slotMaxTime="22:00:00"
        scrollTime="09:00:00"
        // spostandosi di giorno o di settimana l'altezza a cui si guarda resta quella
        scrollTimeReset={false}
        slotDuration="00:30:00"
        slotLabelInterval="01:00:00"
        // riunioni in sovrapposizione affiancate invece che una sopra l'altra
        slotEventOverlap={false}
        slotLabelFormat={{ hour: "2-digit", minute: "2-digit", hour12: false }}
        eventTimeFormat={{ hour: "2-digit", minute: "2-digit", hour12: false }}
        // la pagina pubblica non ha niente da mettere in cima: le festività stanno sotto la data
        allDaySlot={!senzaRiga}
        allDayText=""
        dayHeaderFormat={{ weekday: "short", day: "numeric" }}
        headerToolbar={false}
        events={eventi}
        eventClick={eventClick}
        // spazio libero: un clic sceglie la mezz'ora, trascinando in verticale se ne prende di più; lo toglie la creazione rapida quando si chiude
        selectable={crea}
        selectMirror
        unselectAuto={false}
        selectAllow={dentroUnGiorno}
        select={select}
        eventContent={eventContent}
        datesSet={datesSet}
        dayHeaderClassNames={classiIntestazione}
        dayCellClassNames={classiCella}
        dayHeaderContent={dayHeaderContent}
      />
    </div>
  );
});
