"use client";

import { schedeTimesheet, sigleScheda } from "@/lib/oe/logic";
import { useOe } from "./OeProvider";

/** Le aziende con il loro gestionale: un clic apre il sito in una scheda nuova. */
export function Timesheet() {
  const { pers, mostraAvviso } = useOe();
  const schede = schedeTimesheet(pers.interfaccia.schede_timesheet);
  return (
    <section id="view-timesheet" className="timesheet min-h-0 flex-1 overflow-y-auto rounded-lg border bg-card p-6 shadow-sm">
      <div className="mb-5">
        <h2 className="text-xl font-semibold">Timesheet</h2>
      </div>
      {schede.length === 0 ? (
        <p className="hint text-sm text-muted-foreground">Nessuna scheda. Si aggiungono in personalizzazione.json, voce &quot;schede_timesheet&quot;.</p>
      ) : (
        <div id="ts-grid" className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
          {schede.map((a, i) => (
            <button
              key={i}
              className="ts-card flex items-center gap-3 rounded-xl border bg-background p-3 text-left transition hover:bg-accent hover:shadow-sm"
              title={a.nome}
              onClick={() => (a.url ? window.open(a.url, "_blank", "noopener") : mostraAvviso({ testo: `Manca l'indirizzo di ${a.nome} in personalizzazione.json.`, tipo: "info" }))}
            >
              <span className="ts-logo-box grid size-12 flex-none place-items-center overflow-hidden rounded-lg border bg-white">
                {a.logo ? (
                  // eslint-disable-next-line @next/next/no-img-element
                  <img src={a.logo} alt={a.nome} className="size-full object-contain p-1" />
                ) : (
                  <span className="ts-iniz grid size-full place-items-center text-sm font-bold text-white" style={{ background: a.colore || "#5b5fc7" }}>
                    {sigleScheda(a.nome)}
                  </span>
                )}
              </span>
              <span className="ts-name flex min-w-0 flex-col text-[15px] font-semibold">
                <span className="truncate">{a.nome}</span>
                {a.sotto && <span className="ts-sub text-[11px] font-normal text-muted-foreground">{a.sotto}</span>}
              </span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
