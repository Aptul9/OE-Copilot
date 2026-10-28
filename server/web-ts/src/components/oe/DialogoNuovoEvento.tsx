"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import { chiaveGiorno, daDatetimeLocal, perDatetimeLocal, spostaGiorni } from "@/lib/oe/logic";
import type { Bozza } from "./CreazioneRapida";
import { useOe } from "./OeProvider";

/** `iniziale`: quello che arriva dalla creazione rapida (Altre opzioni), al posto dell'ora tonda successiva. */
type Props = { aperto: boolean; onChiudi: () => void; iniziale?: Bozza | null };

/** Un evento tuo, nel calendario "Altro": sta solo sul server. */
export function DialogoNuovoEvento({ aperto, onChiudi, iniziale }: Props) {
  const { aggiornaCalendari, ricarica } = useOe();
  const [titolo, setTitolo] = useState("");
  const [tutto, setTutto] = useState(false);
  const [inizio, setInizio] = useState("");
  const [fine, setFine] = useState("");
  const [luogo, setLuogo] = useState("");
  const [note, setNote] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);
  const [blocca, setBlocca] = useState(true);

  useEffect(() => {
    if (!aperto) return;
    setLuogo("");
    setNote("");
    setEsito("");
    if (iniziale) {
      // un giorno intero: il campo mostra l'ultimo giorno, la selezione ha la fine esclusiva
      const t = iniziale.tuttoIlGiorno;
      setTitolo(iniziale.titolo);
      setTutto(t);
      setBlocca(iniziale.blocca);
      setInizio(t ? chiaveGiorno(iniziale.inizio) : perDatetimeLocal(iniziale.inizio));
      setFine(t ? chiaveGiorno(spostaGiorni(iniziale.fine, -1)) : perDatetimeLocal(iniziale.fine));
      return;
    }
    // preimposta sull'ora tonda successiva, durata un'ora
    const ora = new Date();
    ora.setMinutes(0, 0, 0);
    ora.setHours(ora.getHours() + 1);
    setTitolo("");
    setTutto(false);
    setBlocca(true);
    setInizio(perDatetimeLocal(ora));
    setFine(perDatetimeLocal(new Date(ora.getTime() + 3600_000)));
  }, [aperto, iniziale]);

  const salva = async () => {
    setEsito("Salvo…");
    setOccupato(true);
    try {
      // l'ora scritta è quella del dispositivo, il server salva l'ora di Roma; un giorno intero è una data, e la fine si salva esclusiva
      // (il giorno dopo l'ultimo, come in iCalendar), mentre il campo mostra l'ultimo giorno
      const [s, f] = tutto ? [inizio, chiaveGiorno(spostaGiorni(new Date(`${fine || inizio}T00:00`), 1))] : [daDatetimeLocal(inizio), daDatetimeLocal(fine)];
      const r = await api("add_local_event", { title: titolo, start: s, end: f, allDay: tutto, location: luogo, notes: note, blocca });
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

  // un giorno intero ha i campi data, e un'ora torna con l'ora tonda delle 9
  const cambiaTutto = (v: boolean) => {
    setTutto(v);
    setInizio((t) => (v ? t.slice(0, 10) : `${t.slice(0, 10)}T09:00`));
    setFine((t) => (v ? t.slice(0, 10) : `${t.slice(0, 10)}T10:00`));
  };

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-event" className="max-h-[86dvh] overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Nuovo evento</DialogTitle>
          <DialogDescription>
            Va nel calendario <b>Altro</b>.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-3 text-sm"
          onSubmit={(e) => {
            e.preventDefault();
            void salva();
          }}
        >
          <label className="flex flex-col gap-1 text-xs font-medium">
            Titolo
            <Input id="ev-title" autoFocus value={titolo} onChange={(e) => setTitolo(e.target.value)} placeholder="es. Dentista" />
          </label>
          <label className="flex items-center gap-2 text-sm">
            <Switch id="ev-allday" checked={tutto} onCheckedChange={cambiaTutto} /> Tutto il giorno
          </label>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label className="flex flex-col gap-1 text-xs font-medium">
              {tutto ? "Dal" : "Inizio"}
              <Input id="ev-start" type={tutto ? "date" : "datetime-local"} value={inizio} onChange={(e) => setInizio(e.target.value)} />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium">
              {tutto ? "Al (compreso)" : "Fine"}
              <Input id="ev-end" type={tutto ? "date" : "datetime-local"} value={fine} onChange={(e) => setFine(e.target.value)} />
            </label>
          </div>
          <div>
            <label className="flex items-center gap-2 text-sm">
              <Switch id="ev-blocca" checked={blocca} onCheckedChange={setBlocca} /> Blocca il tempo
            </label>
            <p className="mt-1 text-xs text-muted-foreground">
              {blocca ? "Nella disponibilità pubblica risulti occupato, anche per giorni interi." : "Solo nel tuo calendario: la disponibilità resta libera."}
            </p>
          </div>
          <label className="flex flex-col gap-1 text-xs font-medium">
            Luogo (facoltativo)
            <Input id="ev-location" value={luogo} onChange={(e) => setLuogo(e.target.value)} placeholder="es. Studio, via Roma 1" />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium">
            Note (facoltativo)
            <Input id="ev-notes" value={note} onChange={(e) => setNote(e.target.value)} />
          </label>
          <span id="ev-result" className="min-h-4 text-xs text-muted-foreground">
            {esito}
          </span>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onChiudi}>
              Annulla
            </Button>
            <Button id="btn-save-event" type="submit" disabled={occupato}>
              Aggiungi
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
