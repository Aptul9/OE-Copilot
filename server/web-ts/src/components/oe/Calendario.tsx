"use client";

import itLocale from "@fullcalendar/core/locales/it";
import type { DatesSetArg, DayHeaderContentArg, EventClickArg, EventContentArg, CalendarOptions } from "@fullcalendar/core";
import dayGridPlugin from "@fullcalendar/daygrid";
import interactionPlugin from "@fullcalendar/interaction";
import listPlugin from "@fullcalendar/list";
import timeGridPlugin from "@fullcalendar/timegrid";
import FullCalendar from "@fullcalendar/react";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { VISTA_3G, chiaveGiorno, iniziali, urlLogo } from "@/lib/oe/logic";
import type { Evento } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import "./calendario.css";
import { useOe } from "./OeProvider";
import { useScorrimentoTreGiorni } from "./useScorrimentoTreGiorni";

export { VISTA_3G };
export const VISTE_VALIDE = ["timeGridDay", VISTA_3G, "timeGridWeek", "dayGridMonth", "listWeek"] as const;
export type Vista = (typeof VISTE_VALIDE)[number];

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
    cambiaVista: (v) => api()?.changeView(v),
  }));

  useScorrimentoTreGiorni(radice, () => (tipoCorrente.current === VISTA_3G ? api() ?? null : null));

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
      tipoCorrente.current = i.view.type;
      setTipo(i.view.type);
      onDatesSet({ start: i.start, end: i.end, tipo: i.view.type, titolo: i.view.title });
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

  const loghi = useMemo<Record<string, string>>(
    () => ({ "Eventi personali": "/logos/personale.png", ...Object.fromEntries(Object.entries(pers.interfaccia.loghi ?? {}).map(([nome, file]) => [nome, urlLogo(file)])) }),
    [pers],
  );
  const sigle = pers.interfaccia.sigle ?? {};

  const eventContent = useCallback(
    (arg: EventContentArg) => {
      const p = arg.event.extendedProps as Evento;
      const tipo = arg.view.type;
      if (tipo === "dayGridMonth" || tipo.startsWith("list")) {
        // mese e agenda: una riga sola, orario e titolo
        const logoRiga = loghi[p.alias || p.account || ""];
        return (
          <div className="ev-line">
            {logoRiga && (
              // eslint-disable-next-line @next/next/no-img-element
              <img className="ev-logo" src={logoRiga} alt="" />
            )}
            {arg.timeText && <span className="ev-time">{arg.timeText} </span>}
            {arg.event.title}
          </div>
        );
      }
      const alias = p.alias || p.account || "";
      const logo = loghi[alias];
      const angolo = arg.event.allDay ? null : logo ? (
        <span className="ev-corner" title={alias}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={logo} alt="" />
        </span>
      ) : alias ? (
        <span className="ev-corner iniziali" style={{ background: p.color }} title={alias}>
          {sigle[alias] || iniziali(alias)}
        </span>
      ) : null;
      // nella vista Giorno e a 3 giorni c'è la larghezza per scrivere anche di che azienda è la riunione
      const azienda = tipo === "timeGridDay" || tipo === VISTA_3G ? alias : "";
      return (
        <>
          {angolo}
          <div className="ev-body">
            {arg.timeText && (
              <div className="ev-meta">
                {arg.timeText}
                {azienda && <span className="ev-az"> · {azienda}</span>}
              </div>
            )}
            <div className="ev-title">{arg.event.title}</div>
          </div>
        </>
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
      // tre giorni che scorrono di uno alla volta, come in Outlook
      [VISTA_3G]: { type: "timeGrid", duration: { days: 3 }, dateIncrement: { days: 1 }, titleFormat: { day: "numeric", month: "long", year: "numeric" } },
      dayGridMonth: { dayHeaderFormat: { weekday: "short" } },
    }),
    [],
  );

  return (
    <div ref={radice} className={cn("calendario h-full", tipo === VISTA_3G && "vista-3g", tipo === "timeGridWeek" && "vista-sett")} style={{ ["--slot-h" as string]: `${zoom}px` }}>
      <FullCalendar
        ref={fc}
        plugins={[dayGridPlugin, timeGridPlugin, listPlugin, interactionPlugin]}
        initialView={vistaIniziale}
        views={viste}
        locale={itLocale}
        firstDay={1}
        nowIndicator
        height="100%"
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
