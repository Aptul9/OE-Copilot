"use client";

import itLocale from "@fullcalendar/core/locales/it";
import type { DatesSetArg, DayHeaderContentArg, EventClickArg, EventContentArg, CalendarOptions } from "@fullcalendar/core";
import dayGridPlugin from "@fullcalendar/daygrid";
import interactionPlugin from "@fullcalendar/interaction";
import listPlugin from "@fullcalendar/list";
import timeGridPlugin from "@fullcalendar/timegrid";
import FullCalendar from "@fullcalendar/react";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { VISTA_3G, chiaveGiorno, iniziali, mappaLoghi, spostaGiorni } from "@/lib/oe/logic";
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

export type ComandiCalendario = {
  oggi: () => void;
  prima: () => void;
  dopo: () => void;
  vaiA: (d: Date) => void;
  cambiaVista: (v: string) => void;
};

type Props = {
  vistaIniziale: string;
  onDatesSet: (info: { start: Date; end: Date; tipo: string; titolo: string }) => void;
  onEventClick: (ev: Evento, titolo: string, inizio: Date | null, fine: Date | null, x: number, y: number) => void;
};

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

export const Calendario = forwardRef<ComandiCalendario, Props>(function Calendario({ vistaIniziale, onDatesSet, onEventClick }, ref) {
  const { eventiGriglia, infoGiorni, pers, zoom } = useOe();
  const fc = useRef<FullCalendar>(null);
  const radice = useRef<HTMLDivElement>(null);
  const [tipo, setTipo] = useState<string>(vistaIniziale);
  const tipoCorrente = useRef<string>(vistaIniziale);
  const api = () => fc.current?.getApi();

  useImperativeHandle(ref, () => ({
    oggi: () => api()?.today(),
    prima: () => api()?.prev(),
    dopo: () => api()?.next(),
    vaiA: (d) => api()?.gotoDate(d),
    cambiaVista: (v) => api()?.changeView(aFc(v)),
  }));

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
      onEventClick(a.event.extendedProps as Evento, a.event.title, a.event.start, a.event.end, a.jsEvent.clientX, a.jsEvent.clientY);
    },
    [onEventClick],
  );

  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  const sigle = pers.interfaccia.sigle ?? {};

  const eventContent = useCallback(
    (arg: EventContentArg) => {
      const p = arg.event.extendedProps as Evento;
      const tipo = daFc(arg.view.type);
      if (tipo === "dayGridMonth" || tipo.startsWith("list")) {
        // mese e agenda: una riga sola, orario e titolo
        const logoRiga = loghi[p.alias || p.account || ""];
        return (
          <div className="ev-line">
            {logoRiga && <ImgLogo key={logoRiga} src={logoRiga} className="ev-logo" alternativa={null} />}
            {arg.timeText && <span className="ev-time">{arg.timeText} </span>}
            {arg.event.title}
          </div>
        );
      }
      const alias = p.alias || p.account || "";
      const logo = loghi[alias];
      const sigla = sigle[alias] || iniziali(alias);
      const angolo = arg.event.allDay ? null : logo ? (
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
  const intero = infoGiorni.tuttoGiorno;

  // Weekend e festività si distinguono a colpo d'occhio; le festività e gli eventi di un giorno
  // intero stanno sotto la data, non come blocco nella griglia.
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
      const chip = intero[g] ?? [];
      return (
        <>
          <span>{arg.text}</span>
          {(festivi[g] || chip.length > 0) && (
            <span className="fc-day-extra">
              {festivi[g] && (
                <span className="fc-festivo-pill" title={festivi[g]}>
                  {festivi[g]}
                </span>
              )}
              {chip.map((c, i) => (
                <span key={i} className="fc-allday-chip" style={{ background: `color-mix(in srgb, ${c.color} 16%, var(--card))`, borderLeftColor: c.color }} title={c.title}>
                  {c.title}
                </span>
              ))}
            </span>
          )}
        </>
      );
    },
    [festivi, intero],
  );

  const viste = useMemo<NonNullable<CalendarOptions["views"]>>(
    () => ({
      // giorno, tre giorni e settimana scorrono sotto il dito come in Outlook: FullCalendar disegna anche i giorni accanto
      [A_FC.timeGridDay]: { type: "timeGrid", visibleRange: intervallo(SCORRE.timeGridDay), dateIncrement: { days: 1 } },
      [VISTA_3G]: { type: "timeGrid", visibleRange: intervallo(SCORRE[VISTA_3G]), dateIncrement: { days: 1 } },
      [A_FC.timeGridWeek]: { type: "timeGrid", visibleRange: intervallo(SCORRE.timeGridWeek), dateIncrement: { weeks: 1 } },
      dayGridMonth: { dayHeaderFormat: { weekday: "short" } },
    }),
    [],
  );

  return (
    <div ref={radice} className={cn("calendario h-full", SCORRE[tipo] && "scorre", tipo === "timeGridWeek" && "vista-sett")} style={{ ["--slot-h" as string]: `${zoom}px` }}>
      <FullCalendar
        ref={fc}
        plugins={[dayGridPlugin, timeGridPlugin, listPlugin, interactionPlugin]}
        initialView={aFc(vistaIniziale)}
        views={viste}
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
        allDaySlot={false}
        dayHeaderFormat={{ weekday: "short", day: "numeric" }}
        headerToolbar={false}
        events={eventiGriglia}
        eventClick={eventClick}
        eventContent={eventContent}
        datesSet={datesSet}
        dayHeaderClassNames={classiIntestazione}
        dayCellClassNames={classiCella}
        dayHeaderContent={dayHeaderContent}
      />
    </div>
  );
});
