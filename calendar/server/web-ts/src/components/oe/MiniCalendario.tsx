"use client";

import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { chiaveGiorno } from "@/lib/oe/logic";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { useOe } from "./OeProvider";

const MESI = ["Gennaio", "Febbraio", "Marzo", "Aprile", "Maggio", "Giugno", "Luglio", "Agosto", "Settembre", "Ottobre", "Novembre", "Dicembre"];
const GIORNI = ["L", "M", "M", "G", "V", "S", "D"];

const stesso = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** Il mese in piccolo: segue il periodo del calendario grande, e un clic su un giorno ci porta lì. */
export function MiniCalendario({ onVaiA }: { onVaiA: (d: Date) => void }) {
  const { periodo, infoGiorni, pubblico, config, salvaConfig } = useOe();
  const [mese, setMese] = useState<Date | null>(null);

  // al primo periodo il mese è quello mostrato; poi lo cambiano le frecce (o un salto del calendario grande)
  const inizio = periodo?.start.getTime();
  useEffect(() => {
    // il mese del punto di mezzo: la vista Mese comincia con i giorni di quello prima
    if (periodo) {
      const metà = new Date((periodo.start.getTime() + periodo.end.getTime()) / 2);
      setMese(new Date(metà.getFullYear(), metà.getMonth(), 1));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inizio]);

  const base = mese ?? new Date(new Date().getFullYear(), new Date().getMonth(), 1);
  const oggi = new Date();
  const primo = new Date(base.getFullYear(), base.getMonth(), 1);
  const scarto = (primo.getDay() + 6) % 7;   // 0 = lunedì
  const giorni = Array.from({ length: 42 }, (_, i) => new Date(base.getFullYear(), base.getMonth(), 1 - scarto + i));
  const vai = (d: number) => setMese(new Date(base.getFullYear(), base.getMonth() + d, 1));

  return (
    <div className="minical px-3">
      <div className="mb-1 flex items-center justify-between">
        <span id="minical-title" className="text-sm font-semibold">
          {MESI[base.getMonth()]} {base.getFullYear()}
        </span>
        <div className="flex">
          <button className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-accent" aria-label="Mese precedente" id="minical-prev" onClick={() => vai(-1)}>
            <ChevronLeftIcon className="size-4" />
          </button>
          <button className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-accent" aria-label="Mese successivo" id="minical-next" onClick={() => vai(1)}>
            <ChevronRightIcon className="size-4" />
          </button>
        </div>
      </div>
      <div id="minical-grid" className="grid grid-cols-7 place-items-center gap-y-0.5 text-center text-xs">
        {GIORNI.map((g, i) => (
          <span key={i} className="mc-dow py-1 text-[11px] text-muted-foreground">
            {g}
          </span>
        ))}
        {giorni.map((d) => {
          const fuori = d.getMonth() !== base.getMonth();
          const selezionato = !!periodo && d >= periodo.start && d < periodo.end;
          const festivo = chiaveGiorno(d) in infoGiorni.festivi;
          return (
            <button
              key={d.getTime()}
              className={cn(
                "mc-day size-7 rounded-full text-[12.5px] hover:bg-accent",
                fuori && "mc-off text-muted-foreground/55",
                festivo && !fuori && "giorno-festivo text-red-600",
                selezionato && "mc-sel bg-accent",
                stesso(d, oggi) && "mc-today bg-primary font-semibold text-primary-foreground hover:bg-primary",
              )}
              onClick={() => onVaiA(d)}
            >
              {d.getDate()}
            </button>
          );
        })}
      </div>
      {!pubblico && (
        <label className="mt-1.5 flex min-h-9 items-center justify-between gap-2 rounded-md text-[12.5px] text-muted-foreground">
          Eventi di tutto il giorno
          <Switch id="minical-tutto-il-giorno" checked={config.nascondi_tutto_il_giorno !== true} onCheckedChange={(v) => void salvaConfig({ nascondi_tutto_il_giorno: !v })} />
        </label>
      )}
    </div>
  );
}
