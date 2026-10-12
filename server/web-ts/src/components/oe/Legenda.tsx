"use client";

import { CheckIcon, SearchIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { Input } from "@/components/ui/input";
import { filtraAziende, iniziali, mappaLoghi, salute } from "@/lib/oe/logic";
import { cn } from "@/lib/utils";
import { ImgLogo } from "./ImgLogo";
import { useOe } from "./OeProvider";

const COLORE_PALLINO: Record<string, string> = {
  good: "bg-emerald-600",
  warn: "bg-amber-500",
  bad: "bg-red-600",
  attesa: "bg-muted-foreground/50 animate-pulse",
};

/** "I miei calendari": una voce per azienda, con interruttore, "only" e il pallino di salute. */
export function Legenda({ onGestisci }: { onGestisci: () => void }) {
  const { gruppi, statoFonti, pallinoInAttesa, accendiAzienda, soloAzienda, gruppiCalendari, gruppiAccesi, soloGruppo, pers } = useOe();
  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  const sigle = pers.interfaccia.sigle ?? {};
  const nomiGruppi = Object.keys(gruppiCalendari);
  const [ricerca, setRicerca] = useState("");
  const visibili = filtraAziende(gruppi, ricerca);
  return (
    <section aria-label="I miei calendari" className="px-1">
      <div className="mb-1 flex items-center justify-between px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
        <span>I miei calendari</span>
        <button id="mycal-tutti" className="rounded px-1.5 py-0.5 text-[11px] normal-case hover:bg-accent hover:text-foreground" title="Mostra tutti i calendari" onClick={() => void soloAzienda("")}>
          Tutti
        </button>
      </div>
      {nomiGruppi.length > 0 && (
        <div id="gruppi" role="group" aria-label="Gruppi di calendari" className="mb-1.5 flex flex-wrap gap-1 px-2">
          {nomiGruppi.map((nome) => {
            const acceso = gruppiAccesi.includes(nome);
            const vuoto = !gruppiCalendari[nome].some((a) => gruppi.some((g) => g.azienda === a));
            return (
              <button
                key={nome}
                type="button"
                data-gruppo={nome}
                aria-pressed={acceso}
                disabled={vuoto}
                title={vuoto ? "Nessun calendario in questo gruppo: assegnali da Gestisci calendari" : acceso ? "Torna a tutti i calendari" : `Mostra solo i calendari del gruppo ${nome}`}
                className={cn(
                  "max-w-full truncate rounded-full border px-2.5 py-0.5 text-[12px] font-medium",
                  acceso ? "border-primary bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
                  vuoto && "cursor-not-allowed opacity-50 hover:bg-transparent",
                )}
                onClick={() => void (acceso ? soloAzienda("") : soloGruppo(nome))}
              >
                {nome}
              </button>
            );
          })}
        </div>
      )}
      {gruppi.length > 1 && (
        <div className="relative mb-1.5 px-2">
          <SearchIcon className="pointer-events-none absolute left-[18px] top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden />
          <Input
            id="mycal-cerca"
            type="search"
            value={ricerca}
            placeholder="Cerca calendari"
            aria-label="Cerca tra i miei calendari"
            autoComplete="off"
            className="h-7 pl-7 text-[13px] md:text-[13px]"
            onChange={(e) => setRicerca(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setRicerca("");
            }}
          />
        </div>
      )}
      <div id="legend" className="flex flex-col">
        {gruppi.length === 0 && <span className="empty px-2 py-1.5 text-xs text-muted-foreground">Nessun calendario. Apri “Gestisci calendari”.</span>}
        {gruppi.length > 0 && visibili.length === 0 && <span className="empty px-2 py-1.5 text-xs text-muted-foreground">Nessun calendario per “{ricerca.trim()}”.</span>}
        {visibili.map((g) => {
          const attivo = g.on > 0;
          const s = salute(statoFonti, g, pallinoInAttesa);
          const logo = loghi[g.azienda];
          const sigla = (
            <span className="grid size-full place-items-center text-[9.5px] font-extrabold leading-none text-white" style={{ background: g.color }}>
              {sigle[g.azienda] || iniziali(g.azienda)}
            </span>
          );
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
                {/* il logo dell'azienda (personalizzazione.json, interfaccia.loghi), o le sue iniziali nel suo colore: si riconosce a colpo d'occhio */}
                <span className="acct-logo grid size-[22px] flex-none place-items-center overflow-hidden rounded-md bg-white ring-1 ring-black/10">
                  {/* se il logo non si carica restano le iniziali, mai l'icona dell'immagine rotta */}
                  {logo ? <ImgLogo key={logo} src={logo} className="size-full object-contain p-[2px]" alternativa={sigla} /> : sigla}
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
