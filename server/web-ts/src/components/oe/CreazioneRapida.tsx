"use client";

import { ClockIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import { chiaveGiorno, daDatetimeLocal, fmtRange, perDatetimeLocal } from "@/lib/oe/logic";
import { cn } from "@/lib/utils";
import type { Selezione } from "./Calendario";
import { useOe } from "./OeProvider";

/** Quello che passa alla finestra completa con "Altre opzioni". */
export type Bozza = { titolo: string; inizio: Date; fine: Date; tuttoIlGiorno: boolean; blocca: boolean };

type Props = {
  selezione: Selezione | null;
  onChiudi: () => void;
  onAltreOpzioni: (b: Bozza) => void;
};

/** Il riquadro che si apre col clic su uno spazio libero, come in Outlook: titolo, l'orario scelto, Salva. Va nel calendario Altro, come
 * Nuovo evento; "Altre opzioni" apre quella finestra con quello che c'è già scritto. Su uno schermo stretto è un riquadro in basso. */
export function CreazioneRapida({ selezione, onChiudi, onAltreOpzioni }: Props) {
  const { aggiornaCalendari, ricarica } = useOe();
  const ref = useRef<HTMLFormElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [titolo, setTitolo] = useState("");
  const [blocca, setBlocca] = useState(true);
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);

  useEffect(() => {
    setTitolo("");
    setBlocca(true);
    setEsito("");
  }, [selezione]);

  useLayoutEffect(() => {
    if (!selezione || !ref.current) return;
    const { offsetWidth: w, offsetHeight: h } = ref.current;
    // accanto al clic, a destra se c'è posto, altrimenti a sinistra; mai fuori dalla finestra
    const destra = selezione.x + 12 + w <= window.innerWidth - 10;
    setPos({
      left: Math.max(10, destra ? selezione.x + 12 : selezione.x - 12 - w),
      top: Math.max(10, Math.min(selezione.y - 24, window.innerHeight - h - 16)),
    });
  }, [selezione]);

  useEffect(() => {
    if (!selezione) return;
    // un clic fuori la chiude: su un altro spazio libero la griglia ne sceglie subito un altro, e il riquadro si riapre lì
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
  }, [selezione, onChiudi]);

  if (!selezione) return null;
  const { inizio, fine, tuttoIlGiorno } = selezione;

  const salva = async () => {
    setEsito("Salvo…");
    setOccupato(true);
    try {
      // come Nuovo evento: l'ora è quella del dispositivo e il server salva quella di Roma; un giorno intero è una data, con la fine esclusiva
      const [s, f] = tuttoIlGiorno ? [chiaveGiorno(inizio), chiaveGiorno(fine)] : [daDatetimeLocal(perDatetimeLocal(inizio)), daDatetimeLocal(perDatetimeLocal(fine))];
      const r = await api("add_local_event", { title: titolo.trim(), start: s, end: f, allDay: tuttoIlGiorno, location: "", notes: "", blocca });
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
        "fixed z-50 flex w-[340px] flex-col gap-3 rounded-lg border bg-popover p-4 text-sm text-popover-foreground shadow-xl",
        "max-sm:!left-2 max-sm:!right-2 max-sm:!top-auto max-sm:bottom-2 max-sm:w-auto",
      )}
      onSubmit={(e) => {
        e.preventDefault();
        if (titolo.trim()) void salva();
      }}
    >
      <Input id="quick-title" autoFocus value={titolo} onChange={(e) => setTitolo(e.target.value)} placeholder="Aggiungi un titolo" className="h-10 text-[15px] font-semibold" />
      <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
        <ClockIcon className="size-4 flex-none" />
        <span id="quick-when" className="font-medium text-foreground">
          {fmtRange(inizio, fine, tuttoIlGiorno)}
        </span>
      </div>
      <label className="flex items-center gap-2 text-[13px]">
        <Switch id="quick-blocca" checked={blocca} onCheckedChange={setBlocca} /> Blocca il tempo
      </label>
      {esito && (
        <span id="quick-result" className="text-xs text-muted-foreground">
          {esito}
        </span>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" size="sm" id="quick-more" onClick={() => onAltreOpzioni({ titolo, inizio, fine, tuttoIlGiorno, blocca })}>
          Altre opzioni
        </Button>
        <Button type="submit" size="sm" id="quick-save" disabled={occupato || !titolo.trim()}>
          Salva
        </Button>
      </div>
    </form>
  );
}
