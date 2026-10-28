"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { azioniEvento, fmtRange } from "@/lib/oe/logic";
import type { Evento } from "@/lib/oe/types";
import { cn } from "@/lib/utils";

export type Scheda = { evento: Evento; titolo: string; inizio: Date | null; fine: Date | null; x: number; y: number };

type Props = {
  scheda: Scheda | null;
  senzaTeams: Set<string>;
  onChiudi: () => void;
  /** `uid` c'è per un evento personale, che si cancella davvero; gli altri si tolgono dalla vista (elimina_evento). */
  onElimina: (evento: Evento, uid: string | null) => void;
};

/** I dettagli di una riunione, vicino al clic; su uno schermo stretto è un riquadro in basso. */
export function SchedaEvento({ scheda, senzaTeams, onChiudi, onElimina }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!scheda || !ref.current) return;
    const h = ref.current.offsetHeight;
    setPos({
      left: Math.max(10, Math.min(scheda.x + 8, window.innerWidth - 322)),
      top: Math.max(10, Math.min(scheda.y, window.innerHeight - h - 16)),
    });
  }, [scheda]);

  useEffect(() => {
    if (!scheda) return;
    // si chiude con un clic fuori (non su un altro evento, che la riapre) o con Esc
    const fuori = (e: PointerEvent) => {
      const t = e.target as HTMLElement;
      if (!ref.current?.contains(t) && !t.closest(".fc-event")) onChiudi();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onChiudi();
    document.addEventListener("pointerdown", fuori);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", fuori);
      document.removeEventListener("keydown", esc);
    };
  }, [scheda, onChiudi]);

  if (!scheda) return null;
  const p = scheda.evento;
  const a = azioniEvento(p, senzaTeams);
  const apri = (url: string) => window.open(url, "_blank", "noopener");

  return (
    <div
      ref={ref}
      id="event-pop"
      role="dialog"
      aria-label={scheda.titolo}
      style={pos ?? { left: -9999, top: 0 }}
      className={cn(
        "fixed z-50 w-[310px] rounded-lg border bg-popover p-4 text-popover-foreground shadow-xl",
        // schermo stretto: riquadro a tutta larghezza in basso
        "max-sm:!left-2 max-sm:!right-2 max-sm:!top-auto max-sm:bottom-2 max-sm:w-auto",
      )}
    >
      <h4 className="mb-2.5 flex items-start gap-2 text-[15px] font-semibold leading-snug">
        <span className="mt-1.5 size-2.5 flex-none rounded-full" style={{ background: p.color }} />
        <span className="min-w-0 break-words">{scheda.titolo}</span>
      </h4>
      <div className="mb-1.5 text-[12.5px] leading-snug text-muted-foreground">
        <b className="font-semibold text-foreground">{fmtRange(scheda.inizio, scheda.fine, p.allDay)}</b>
      </div>
      <div className="mb-1.5">
        <span className="inline-block rounded-full px-2.5 py-0.5 text-[11px] font-semibold text-white" style={{ background: p.color }}>
          {p.alias || p.account}
        </span>
      </div>
      {p.location && <Riga etichetta="Luogo">{p.location}</Riga>}
      {p.inoltrata_da ? (
        // copia messa da Power Automate: come organizzatore risulteresti tu, che qui non direbbe niente
        <Riga etichetta="Origine">copia dal calendario {p.inoltrata_da}</Riga>
      ) : (
        p.organizer && <Riga etichetta="Organizza">{p.organizer}</Riga>
      )}
      {p.is_local && !!p.gruppi?.length && <Riga etichetta="A chi">{p.gruppi.join(", ")}</Riga>}
      {p.is_local && p.blocca !== undefined && <Riga etichetta="Blocca">{p.blocca ? "Sì, risulti occupato" : "No, la disponibilità resta libera"}</Riga>}
      <div className="mt-3 flex flex-wrap gap-2">
        {a.partecipa && (
          <Button size="sm" className="join" onClick={() => apri(a.partecipa!)}>
            Partecipa
          </Button>
        )}
        {a.apriInTeams && (
          <Button size="sm" variant="outline" onClick={() => { apri(a.apriInTeams!.url); onChiudi(); }}>
            Apri in Teams
          </Button>
        )}
        <Button id="ev-elimina" size="sm" variant="outline" className="text-destructive" onClick={() => onElimina(p, a.elimina)}>
          Elimina
        </Button>
      </div>
    </div>
  );
}

function Riga({ etichetta, children }: { etichetta: string; children: React.ReactNode }) {
  return (
    <div className="mb-1.5 text-[12.5px] leading-snug text-muted-foreground">
      <span className="inline-block min-w-[62px] text-[11px] uppercase tracking-wide opacity-70">{etichetta}</span> {children}
    </div>
  );
}
