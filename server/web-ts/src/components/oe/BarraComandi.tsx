"use client";

import { ChevronLeftIcon, ChevronRightIcon, MinusIcon, PanelLeftIcon, PlusIcon, RefreshCwIcon, RotateCcwIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { capitalizza } from "@/lib/oe/logic";
import { VISTA_3G } from "./Calendario";
import { useOe, ZOOM_DEFAULT, ZOOM_MAX, ZOOM_MIN } from "./OeProvider";

export const ETICHETTE_VISTE: { valore: string; testo: string }[] = [
  { valore: "timeGridDay", testo: "Giorno" },
  { valore: VISTA_3G, testo: "3 giorni" },
  { valore: "timeGridWeek", testo: "Settimana" },
  { valore: "dayGridMonth", testo: "Mese" },
  { valore: "listWeek", testo: "Agenda" },
];

type Props = {
  titolo: string;
  vista: string;
  stretto: boolean;
  telefono: boolean;
  onPannello: () => void;
  onOggi: () => void;
  onPrima: () => void;
  onDopo: () => void;
  onVista: (v: string) => void;
};

/** La barra sopra il calendario: periodo, sincronizzazione, zoom e vista. */
export function BarraComandi({ titolo, vista, stretto, telefono, onPannello, onOggi, onPrima, onDopo, onVista }: Props) {
  const { forzaSync, sincronizzando, etichetta, zoom, cambiaZoom } = useOe();
  // su uno schermo stretto la settimana non sta; su un telefono nemmeno i tre giorni
  const viste = ETICHETTE_VISTE.filter((v) => !(stretto && v.valore === "timeGridWeek") && !(telefono && v.valore === VISTA_3G));
  return (
    <header className="cmdbar flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
      <div className="flex min-w-0 items-center gap-1.5">
        <Button variant="ghost" size="icon" id="sidepane-toggle" className="lg:inline-flex" title="Mostra o nascondi il pannello" aria-label="Pannello" onClick={onPannello}>
          <PanelLeftIcon />
        </Button>
        <Button variant="outline" id="btn-today" onClick={onOggi}>
          Oggi
        </Button>
        <div className="flex">
          <Button variant="ghost" size="icon" id="btn-prev" title="Periodo precedente" aria-label="Precedente" onClick={onPrima}>
            <ChevronLeftIcon />
          </Button>
          <Button variant="ghost" size="icon" id="btn-next" title="Periodo successivo" aria-label="Successivo" onClick={onDopo}>
            <ChevronRightIcon />
          </Button>
        </div>
        <h1 id="range-label" className="ml-1 min-w-0 truncate text-base font-semibold sm:text-lg">
          {titolo ? capitalizza(titolo) : "Calendario"}
        </h1>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" id="btn-sync" className={cn(sincronizzando && "is-syncing")} title="Sincronizza adesso" onClick={forzaSync}>
          <RefreshCwIcon className={cn(sincronizzando && "animate-spin")} />
          <span id="sync-status" className="sync-text max-sm:sr-only">
            {etichetta}
          </span>
        </Button>
        <div id="zoom-switch" className="hidden items-center md:flex">
          <Button variant="ghost" size="icon" id="btn-zoom-out" title="Rimpicciolisci il calendario" aria-label="Rimpicciolisci" disabled={zoom <= ZOOM_MIN} onClick={() => cambiaZoom(-1)}>
            <MinusIcon />
          </Button>
          <Button variant="ghost" size="icon" id="btn-zoom-in" title="Ingrandisci il calendario" aria-label="Ingrandisci" disabled={zoom >= ZOOM_MAX} onClick={() => cambiaZoom(1)}>
            <PlusIcon />
          </Button>
          <Button variant="ghost" size="icon" id="btn-zoom-reset" title="Ripristina zoom" aria-label="Ripristina zoom" disabled={zoom === ZOOM_DEFAULT} onClick={() => cambiaZoom(0)}>
            <RotateCcwIcon />
          </Button>
        </div>
        {stretto ? (
          <Select value={viste.some((v) => v.valore === vista) ? vista : viste[0].valore} onValueChange={onVista}>
            <SelectTrigger id="view-select" className="w-32" aria-label="Vista">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {viste.map((v) => (
                <SelectItem key={v.valore} value={v.valore}>
                  {v.testo}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <div id="view-switch" className="segmented inline-flex rounded-lg border p-0.5">
            {viste.map((v) => (
              <button
                key={v.valore}
                data-view={v.valore}
                aria-pressed={vista === v.valore}
                className={cn("rounded-md px-3 py-1 text-[13px] font-medium", vista === v.valore ? "is-active bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground")}
                onClick={() => onVista(v.valore)}
              >
                {v.testo}
              </button>
            ))}
          </div>
        )}
      </div>
    </header>
  );
}
