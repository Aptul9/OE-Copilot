"use client";

import { CheckIcon } from "lucide-react";
import { salute } from "@/lib/oe/logic";
import { cn } from "@/lib/utils";
import { useOe } from "./OeProvider";

const COLORE_PALLINO: Record<string, string> = {
  good: "bg-emerald-600",
  warn: "bg-amber-500",
  bad: "bg-red-600",
  attesa: "bg-muted-foreground/50 animate-pulse",
};

/** "I miei calendari": una voce per azienda, con interruttore, "only" e il pallino di salute. */
export function Legenda({ onGestisci }: { onGestisci: () => void }) {
  const { gruppi, statoFonti, pallinoInAttesa, accendiAzienda, soloAzienda } = useOe();
  return (
    <section aria-label="I miei calendari" className="px-1">
      <div className="mb-1 flex items-center justify-between px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        <span>I miei calendari</span>
        <button id="mycal-tutti" className="rounded px-1.5 py-0.5 text-[11px] normal-case hover:bg-accent hover:text-foreground" title="Mostra tutti i calendari" onClick={() => void soloAzienda("")}>
          Tutti
        </button>
      </div>
      <div id="legend" className="flex flex-col">
        {gruppi.length === 0 && <span className="empty px-2 py-1.5 text-xs text-muted-foreground">Nessun calendario. Apri “Gestisci calendari”.</span>}
        {gruppi.map((g) => {
          const attivo = g.on > 0;
          const s = salute(statoFonti, g, pallinoInAttesa);
          return (
            <div key={g.azienda} className={cn("acct group/acct flex items-center rounded-md hover:bg-accent", attivo ? "on" : "off")} data-azienda={g.azienda} style={{ ["--c" as string]: g.color }}>
              <button
                className="flex min-w-0 flex-1 items-center gap-2.5 px-2 py-1.5 text-left text-[13px]"
                title={g.caselle.join(" · ")}
                aria-pressed={attivo}
                onClick={() => void accendiAzienda(g.azienda, !attivo)}
              >
                <span
                  className="acct-box grid size-[18px] flex-none place-items-center rounded-[4px] border-2 text-white"
                  style={{ borderColor: g.color, background: attivo ? g.color : "transparent" }}
                >
                  {attivo && <CheckIcon className="size-3.5" strokeWidth={3} />}
                </span>
                <span className="acct-name truncate">{g.azienda}</span>
              </button>
              <button
                className="acct-only mr-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground opacity-0 hover:bg-background hover:text-foreground focus-visible:opacity-100 group-hover/acct:opacity-100 max-md:opacity-100"
                title={`Mostra solo ${g.azienda}`}
                onClick={() => void soloAzienda(g.azienda)}
              >
                only
              </button>
              <span className={cn("acct-health mr-2.5 size-2.5 flex-none rounded-full", s.classe, COLORE_PALLINO[s.classe])} title={s.titolo} />
            </div>
          );
        })}
      </div>
      <button
        id="mycal-all"
        className="mt-1 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12.5px] font-semibold text-muted-foreground hover:bg-accent hover:text-foreground"
        onClick={onGestisci}
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="M4 7.5h9M4 12h9M4 16.5h9M16.5 7l1.6 1.6L21 5.9M16.5 15.6l1.6 1.6L21 14.5" />
        </svg>
        Gestisci calendari
      </button>
    </section>
  );
}
