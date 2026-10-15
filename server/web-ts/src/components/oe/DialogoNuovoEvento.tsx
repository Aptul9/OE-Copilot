"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import { perDatetimeLocal } from "@/lib/oe/logic";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void };

/** Un evento tuo, nel calendario "Altro": sta solo sul server. */
export function DialogoNuovoEvento({ aperto, onChiudi }: Props) {
  const { aggiornaCalendari, ricarica } = useOe();
  const [titolo, setTitolo] = useState("");
  const [tutto, setTutto] = useState(false);
  const [inizio, setInizio] = useState("");
  const [fine, setFine] = useState("");
  const [luogo, setLuogo] = useState("");
  const [note, setNote] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState(false);

  useEffect(() => {
    if (!aperto) return;
    // preimposta sull'ora tonda successiva, durata un'ora
    const ora = new Date();
    ora.setMinutes(0, 0, 0);
    ora.setHours(ora.getHours() + 1);
    setTitolo("");
    setLuogo("");
    setNote("");
    setTutto(false);
    setInizio(perDatetimeLocal(ora));
    setFine(perDatetimeLocal(new Date(ora.getTime() + 3600_000)));
    setEsito("");
  }, [aperto]);

  const salva = async () => {
    setEsito("Salvo…");
    setOccupato(true);
    try {
      const r = await api("add_local_event", { title: titolo, start: inizio, end: fine, allDay: tutto, location: luogo, notes: note });
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
            <Switch id="ev-allday" checked={tutto} onCheckedChange={setTutto} /> Tutto il giorno
          </label>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <label className="flex flex-col gap-1 text-xs font-medium">
              Inizio
              <Input id="ev-start" type="datetime-local" value={inizio} onChange={(e) => setInizio(e.target.value)} />
            </label>
            <label className="flex flex-col gap-1 text-xs font-medium">
              Fine
              <Input id="ev-end" type="datetime-local" value={fine} onChange={(e) => setFine(e.target.value)} />
            </label>
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
