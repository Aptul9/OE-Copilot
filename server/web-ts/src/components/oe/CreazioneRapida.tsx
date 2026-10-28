"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import { chiaveGiorno, daDatetimeLocal, perDatetimeLocal, spostaGiorni } from "@/lib/oe/logic";
import { cn } from "@/lib/utils";
import type { Selezione } from "./Calendario";
import { useOe } from "./OeProvider";
import { SceltaGruppi, gruppiDiPartenza } from "./SceltaGruppi";

/** Quello che passa alla finestra completa con "Altre opzioni". */
export type Bozza = { titolo: string; inizio: Date; fine: Date; tuttoIlGiorno: boolean; blocca: boolean; gruppi: string[] };

type Props = {
  selezione: Selezione | null;
  onChiudi: () => void;
  onAltreOpzioni: (b: Bozza) => void;
  /** Data e orari cambiati nei campi: la griglia sposta lo spazio scelto lì. */
  onCambia: (inizio: Date, fine: Date, tuttoIlGiorno: boolean) => void;
};

/** I campi come si scrivono: giorno ("YYYY-MM-DD"), ore ("HH:MM") e, per i giorni interi, l'ultimo giorno compreso. */
type Campi = { tutto: boolean; giorno: string; dalle: string; alle: string; finoAl: string };

const ora = (d: Date) => perDatetimeLocal(d).slice(11);
const campiDa = (s: Selezione): Campi => ({
  tutto: s.tuttoIlGiorno,
  giorno: chiaveGiorno(s.inizio),
  dalle: s.tuttoIlGiorno ? "09:00" : ora(s.inizio),
  alle: s.tuttoIlGiorno ? "09:30" : ora(s.fine),
  // la selezione di giorni interi ha la fine esclusiva (il giorno dopo l'ultimo)
  finoAl: chiaveGiorno(s.tuttoIlGiorno ? spostaGiorni(s.fine, -1) : s.inizio),
});
/** Gli istanti dei campi, con la fine esclusiva per i giorni interi; null se un campo è vuoto o non è una data. */
function istanti(c: Campi): { inizio: Date; fine: Date } | null {
  const inizio = new Date(`${c.giorno}T${c.tutto ? "00:00" : c.dalle}`);
  const fine = c.tutto ? spostaGiorni(new Date(`${c.finoAl}T00:00`), 1) : new Date(`${c.giorno}T${c.alle}`);
  return Number.isNaN(inizio.getTime()) || Number.isNaN(fine.getTime()) ? null : { inizio, fine };
}

/** Il riquadro che si apre col clic su uno spazio libero, come in Outlook: titolo, giorno e orari (quelli del clic, da cambiare a piacere),
 * Salva. Va nel calendario Altro, come Nuovo evento; "Altre opzioni" apre quella finestra con quello che c'è già scritto. Su uno schermo
 * stretto è un riquadro in basso. */
export function CreazioneRapida({ selezione, onChiudi, onAltreOpzioni, onCambia }: Props) {
  const { aggiornaCalendari, ricarica, gruppiAccesi } = useOe();
  const ref = useRef<HTMLFormElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [titolo, setTitolo] = useState("");
  const [campi, setCampi] = useState<Campi | null>(null);
  const [blocca, setBlocca] = useState(true);
  const [gruppi, setGruppi] = useState<string[]>([]);
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);

  // una selezione nuova (clic sulla griglia) riparte da capo; quelle che arrivano dai campi (onCambia) hanno già i campi giusti
  const daiCampi = useRef(false);
  useEffect(() => {
    if (daiCampi.current) {
      daiCampi.current = false;
      return;
    }
    setCampi(selezione ? campiDa(selezione) : null);
    if (!selezione) return;
    setTitolo("");
    setBlocca(true);
    setGruppi(gruppiDiPartenza(gruppiAccesi));
    setEsito("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selezione]);

  // la posizione si calcola all'apertura, non a ogni orario cambiato: il riquadro non salta sotto il mouse
  // (pronto: il riquadro c'è solo quando ha i campi, un giro dopo la selezione; misurato prima non esisterebbe)
  const aperto = !!selezione;
  const pronto = aperto && !!campi;
  const x = selezione?.x;
  const y = selezione?.y;
  useLayoutEffect(() => {
    if (!pronto || x === undefined || y === undefined || !ref.current) return;
    const { offsetWidth: w, offsetHeight: h } = ref.current;
    // accanto al clic, a destra se c'è posto, altrimenti a sinistra; mai fuori dalla finestra
    const destra = x + 12 + w <= window.innerWidth - 10;
    setPos({
      left: Math.max(10, destra ? x + 12 : x - 12 - w),
      top: Math.max(10, Math.min(y - 24, window.innerHeight - h - 16)),
    });
  }, [pronto, x, y]);

  useEffect(() => {
    if (!aperto) return;
    // un clic fuori la chiude: su un altro spazio libero la griglia ne sceglie subito un altro, e il riquadro si riapre lì. I menu dei campi
    // data e ora del browser non sono nel documento: non contano come fuori
    const fuori = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onChiudi();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onChiudi();
    document.addEventListener("pointerdown", fuori);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", fuori);
      document.removeEventListener("keydown", esc);
    };
  }, [aperto, onChiudi]);

  if (!selezione || !campi) return null;
  const t = istanti(campi);
  const errore = !t ? "Data o orario non validi." : t.fine <= t.inizio ? "La fine viene prima dell'inizio." : "";

  // come Outlook: spostando l'inizio la durata resta quella, e la fine si sposta con lui
  const cambia = (nuovi: Partial<Campi>) => {
    const c = { ...campi, ...nuovi };
    if (nuovi.dalle !== undefined && !c.tutto) {
      const prima = istanti(campi);
      const dopo = new Date(`${c.giorno}T${nuovi.dalle}`);
      if (prima && !Number.isNaN(dopo.getTime()) && prima.fine > prima.inizio) {
        const fine = new Date(dopo.getTime() + (prima.fine.getTime() - prima.inizio.getTime()));
        // la fine resta nello stesso giorno: oltre mezzanotte si ferma alle 23:59
        c.alle = chiaveGiorno(fine) === c.giorno ? ora(fine) : "23:59";
      }
    }
    if (nuovi.giorno !== undefined && c.tutto && c.finoAl < c.giorno) c.finoAl = c.giorno;
    setCampi(c);
    const i = istanti(c);
    if (i && i.fine > i.inizio) {
      daiCampi.current = true;
      onCambia(i.inizio, i.fine, c.tutto);
    }
  };

  const salva = async () => {
    if (!t || errore) return;
    setEsito("Salvo…");
    setOccupato(true);
    try {
      // come Nuovo evento: l'ora è quella del dispositivo e il server salva quella di Roma; un giorno intero è una data, con la fine esclusiva
      const [s, f] = campi.tutto ? [chiaveGiorno(t.inizio), chiaveGiorno(t.fine)] : [daDatetimeLocal(perDatetimeLocal(t.inizio)), daDatetimeLocal(perDatetimeLocal(t.fine))];
      const r = await api("add_local_event", { title: titolo.trim(), start: s, end: f, allDay: campi.tutto, location: "", notes: "", blocca, gruppi });
      if (r.ok) {
        onChiudi();
        await aggiornaCalendari();
        ricarica();
      } else {
        setEsito(r.error ?? "Non riuscito.");
      }
    } catch (e) {
      setEsito(e instanceof Error ? e.message : String(e));
    } finally {
      setOccupato(false);
    }
  };

  return (
    <form
      ref={ref}
      id="quick-event"
      role="dialog"
      aria-label="Nuovo evento"
      style={pos ?? { left: -9999, top: 0 }}
      className={cn(
        "fixed z-50 flex w-[380px] flex-col gap-3 rounded-lg border bg-popover p-4 text-sm text-popover-foreground shadow-xl",
        "max-sm:!left-2 max-sm:!right-2 max-sm:!top-auto max-sm:bottom-2 max-sm:w-auto",
      )}
      onSubmit={(e) => {
        e.preventDefault();
        if (titolo.trim()) void salva();
      }}
    >
      <Input id="quick-title" autoFocus value={titolo} onChange={(e) => setTitolo(e.target.value)} placeholder="Aggiungi un titolo" className="h-10 text-[15px] font-semibold" />
      {campi.tutto ? (
        <div className="grid grid-cols-2 gap-2">
          <label className="flex flex-col gap-1 text-xs font-medium">
            Dal
            <Input id="quick-day" type="date" value={campi.giorno} onChange={(e) => cambia({ giorno: e.target.value })} />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium">
            Al (compreso)
            <Input id="quick-day-end" type="date" value={campi.finoAl} min={campi.giorno} onChange={(e) => cambia({ finoAl: e.target.value })} />
          </label>
        </div>
      ) : (
        <div className="grid grid-cols-[minmax(0,1fr)_92px_92px] items-end gap-2">
          <label className="flex flex-col gap-1 text-xs font-medium">
            Giorno
            <Input id="quick-day" type="date" value={campi.giorno} onChange={(e) => cambia({ giorno: e.target.value })} />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium">
            Dalle
            <Input id="quick-start" type="time" step={300} value={campi.dalle} onChange={(e) => cambia({ dalle: e.target.value })} className="px-2" />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium">
            Alle
            <Input id="quick-end" type="time" step={300} value={campi.alle} onChange={(e) => cambia({ alle: e.target.value })} className="px-2" />
          </label>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-[13px]">
        <label className="flex items-center gap-2">
          <Switch id="quick-allday" checked={campi.tutto} onCheckedChange={(v) => cambia({ tutto: v, finoAl: campi.giorno })} /> Tutto il giorno
        </label>
        <label className="flex items-center gap-2">
          <Switch id="quick-blocca" checked={blocca} onCheckedChange={setBlocca} /> Blocca il tempo
        </label>
      </div>
      <SceltaGruppi id="quick-gruppi" scelti={gruppi} onCambia={setGruppi} />
      {(errore || esito) && (
        <span id="quick-result" className={cn("text-xs", errore ? "text-destructive" : "text-muted-foreground")}>
          {errore || esito}
        </span>
      )}
      <div className="flex justify-end gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          id="quick-more"
          onClick={() => onAltreOpzioni({ titolo, inizio: t?.inizio ?? selezione.inizio, fine: t?.fine ?? selezione.fine, tuttoIlGiorno: campi.tutto, blocca, gruppi })}
        >
          Altre opzioni
        </Button>
        <Button type="submit" size="sm" id="quick-save" disabled={occupato || !titolo.trim() || !!errore}>
          Salva
        </Button>
      </div>
    </form>
  );
}
