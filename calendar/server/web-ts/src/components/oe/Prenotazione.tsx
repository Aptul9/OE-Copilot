"use client";

import { CalendarCheckIcon, DownloadIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { chiaveGiorno, daDatetimeLocal, fmtRange, perDatetimeLocal } from "@/lib/oe/logic";
import { aRoma, fuoriRegola } from "@/lib/prenota-regole";
import { cn } from "@/lib/utils";
import type { Selezione } from "./Calendario";
import { useOe } from "./OeProvider";
import { useRiquadro } from "./useRiquadro";

type Props = {
  selezione: Selezione | null;
  onChiudi: () => void;
  /** Giorno e orari cambiati nei campi: la griglia sposta lo spazio scelto lì. */
  onCambia: (inizio: Date, fine: Date, tuttoIlGiorno: boolean) => void;
};

type Campi = { giorno: string; dalle: string; alle: string };
type Dati = { nome: string; email: string; azienda: string; argomento: string; sito: string };
const VUOTI: Dati = { nome: "", email: "", azienda: "", argomento: "", sito: "" };

const ora = (d: Date) => perDatetimeLocal(d).slice(11);
const campiDa = (s: Selezione): Campi => ({ giorno: chiaveGiorno(s.inizio), dalle: ora(s.inizio), alle: ora(s.fine) });
function istanti(c: Campi): { inizio: Date; fine: Date } | null {
  const inizio = new Date(`${c.giorno}T${c.dalle}`);
  const fine = new Date(`${c.giorno}T${c.alle}`);
  return Number.isNaN(inizio.getTime()) || Number.isNaN(fine.getTime()) ? null : { inizio, fine };
}
const EMAIL = /^[^\s@<>()",;:]+@[^\s@<>()",;:]+\.[^\s@<>()",;:]{2,}$/;

/** L'invito da aprire con il proprio calendario (Outlook, Google, Apple): l'email non la mandiamo, il server non ne ha. */
function invito(inizio: Date, fine: Date, titolo: string, descrizione: string): string {
  const utc = (d: Date) => d.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}/, "");
  const testo = (t: string) => t.replace(/\\/g, "\\\\").replace(/;/g, "\\;").replace(/,/g, "\\,").replace(/\r?\n/g, "\\n");
  return [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//Everbright//Meet//IT",
    "METHOD:PUBLISH",
    "BEGIN:VEVENT",
    `UID:${crypto.randomUUID()}@meet.everbright.it`,
    `DTSTAMP:${utc(new Date())}`,
    `DTSTART:${utc(inizio)}`,
    `DTEND:${utc(fine)}`,
    `SUMMARY:${testo(titolo)}`,
    `DESCRIPTION:${testo(descrizione)}`,
    "END:VEVENT",
    "END:VCALENDAR",
    "",
  ].join("\r\n");
}

/** La prenotazione di un meeting dalla pagina pubblica: il riquadro che si apre col clic su uno spazio libero, come la creazione rapida del
 * calendario. Giorno e orari a quarti d'ora, nome, email, azienda e argomento; il server ricontrolla regole e spazio libero
 * (/api/prenota, lib/prenota.ts). Fatta, si scarica l'invito. */
export function Prenotazione({ selezione, onChiudi, onCambia }: Props) {
  const { ricarica, gruppoPubblico } = useOe();
  const ref = useRef<HTMLFormElement>(null);
  const [campi, setCampi] = useState<Campi | null>(null);
  const [dati, setDati] = useState<Dati>(VUOTI);
  const [esito, setEsito] = useState("");
  const [inCorso, setInCorso] = useState(false);
  const [fatta, setFatta] = useState<{ inizio: Date; fine: Date } | null>(null);

  // una selezione nuova riparte da capo, tenendo nome e contatti già scritti; quelle che arrivano dai campi hanno già i campi giusti
  const daiCampi = useRef(false);
  useEffect(() => {
    if (daiCampi.current) {
      daiCampi.current = false;
      return;
    }
    setCampi(selezione ? campiDa(selezione) : null);
    if (!selezione) return;
    setEsito("");
    setFatta(null);
  }, [selezione]);

  const aperto = !!selezione;
  const pos = useRiquadro(ref, { aperto, pronto: aperto && !!campi, x: selezione?.x, y: selezione?.y, onChiudi });

  if (!selezione || !campi) return null;
  const t = istanti(campi);
  const regola = !t ? "Data o orario non validi." : t.fine <= t.inizio ? "La fine viene prima dell'inizio." : fuoriRegola(aRoma(t.inizio), aRoma(t.fine), Date.now());
  const completo = dati.nome.trim().length >= 2 && EMAIL.test(dati.email.trim());

  // come Outlook: spostando l'inizio la durata resta quella, e la fine si sposta con lui
  const cambia = (nuovi: Partial<Campi>) => {
    const c = { ...campi, ...nuovi };
    if (nuovi.dalle !== undefined) {
      const prima = istanti(campi);
      const dopo = new Date(`${c.giorno}T${nuovi.dalle}`);
      if (prima && !Number.isNaN(dopo.getTime()) && prima.fine > prima.inizio) {
        const fine = new Date(dopo.getTime() + (prima.fine.getTime() - prima.inizio.getTime()));
        c.alle = chiaveGiorno(fine) === c.giorno ? ora(fine) : "23:59";
      }
    }
    setCampi(c);
    const i = istanti(c);
    if (i && i.fine > i.inizio) {
      daiCampi.current = true;
      onCambia(i.inizio, i.fine, false);
    }
  };
  const scrivi = (k: keyof Dati) => (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setDati({ ...dati, [k]: e.target.value });

  const prenota = async () => {
    if (!t || regola || !completo) return;
    setEsito("Prenoto…");
    setInCorso(true);
    try {
      const r = await fetch("/api/prenota", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ inizio: daDatetimeLocal(perDatetimeLocal(t.inizio)), fine: daDatetimeLocal(perDatetimeLocal(t.fine)), ...dati, gruppo: gruppoPubblico }),
      });
      const j = await r.json().catch(() => ({}));
      if (r.ok && j.ok) {
        setFatta(t);
        setEsito("");
        ricarica();
      } else {
        setEsito((j && j.error) || `Non riuscito (HTTP ${r.status}).`);
        // lo spazio è stato preso nel frattempo: la griglia si rilegge e lo mostra occupato
        if (r.status === 409) ricarica();
      }
    } catch {
      setEsito("Connessione assente: riprova.");
    } finally {
      setInCorso(false);
    }
  };

  const scarica = () => {
    if (!fatta) return;
    const titolo = `Meeting con Everbright${gruppoPubblico ? ` (${gruppoPubblico})` : ""}`;
    const descrizione = [dati.argomento.trim() && `Argomento: ${dati.argomento.trim()}`, `Prenotato da ${dati.nome.trim()}${dati.azienda.trim() ? ` (${dati.azienda.trim()})` : ""}, ${dati.email.trim()}`].filter(Boolean).join("\n");
    const url = URL.createObjectURL(new Blob([invito(fatta.inizio, fatta.fine, titolo, descrizione)], { type: "text/calendar;charset=utf-8" }));
    const a = Object.assign(document.createElement("a"), { href: url, download: "meeting-everbright.ics" });
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };

  const classeRiquadro = cn(
    "fixed z-50 flex w-[400px] flex-col gap-3 rounded-lg border bg-popover p-4 text-sm text-popover-foreground shadow-xl",
    "max-sm:!left-2 max-sm:!right-2 max-sm:!top-auto max-sm:bottom-2 max-sm:max-h-[85dvh] max-sm:w-auto max-sm:overflow-y-auto",
  );

  if (fatta) {
    return (
      <form ref={ref} id="prenota" role="dialog" aria-label="Meeting prenotato" style={pos ?? { left: -9999, top: 0 }} className={classeRiquadro} onSubmit={(e) => e.preventDefault()}>
        <div className="flex items-start gap-3">
          <CalendarCheckIcon className="mt-0.5 size-6 flex-none text-primary" />
          <div className="flex flex-col gap-1">
            <span id="prenota-fatta" className="text-[15px] font-semibold">Meeting prenotato</span>
            <span className="text-muted-foreground">{fmtRange(fatta.inizio, fatta.fine, false)}</span>
            <span className="text-xs text-muted-foreground">Ti scriveremo a {dati.email.trim()} con i dettagli per collegarti.</span>
          </div>
        </div>
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" size="sm" onClick={onChiudi}>
            Chiudi
          </Button>
          <Button type="button" size="sm" id="prenota-ics" onClick={scarica}>
            <DownloadIcon /> Aggiungi al calendario
          </Button>
        </div>
      </form>
    );
  }

  return (
    <form
      ref={ref}
      id="prenota"
      role="dialog"
      aria-label="Prenota un meeting"
      style={pos ?? { left: -9999, top: 0 }}
      className={classeRiquadro}
      onSubmit={(e) => {
        e.preventDefault();
        void prenota();
      }}
    >
      <span className="text-[15px] font-semibold">Prenota un meeting{gruppoPubblico && <span className="text-muted-foreground"> con {gruppoPubblico}</span>}</span>
      <div className="grid grid-cols-[minmax(0,1fr)_92px_92px] items-end gap-2">
        <label className="flex flex-col gap-1 text-xs font-medium">
          Giorno
          <Input id="prenota-giorno" type="date" value={campi.giorno} onChange={(e) => cambia({ giorno: e.target.value })} />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium">
          Dalle
          <Input id="prenota-dalle" type="time" step={900} value={campi.dalle} onChange={(e) => cambia({ dalle: e.target.value })} className="px-2" />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium">
          Alle
          <Input id="prenota-alle" type="time" step={900} value={campi.alle} onChange={(e) => cambia({ alle: e.target.value })} className="px-2" />
        </label>
      </div>
      <div className="grid grid-cols-2 gap-2 max-[420px]:grid-cols-1">
        <label className="flex flex-col gap-1 text-xs font-medium">
          Nome e cognome
          <Input id="prenota-nome" autoFocus autoComplete="name" required maxLength={100} value={dati.nome} onChange={scrivi("nome")} />
        </label>
        <label className="flex flex-col gap-1 text-xs font-medium">
          Email
          <Input id="prenota-email" type="email" autoComplete="email" required maxLength={254} value={dati.email} onChange={scrivi("email")} />
        </label>
      </div>
      <label className="flex flex-col gap-1 text-xs font-medium">
        Azienda <span className="sr-only">(facoltativa)</span>
        <Input id="prenota-azienda" autoComplete="organization" maxLength={100} value={dati.azienda} onChange={scrivi("azienda")} placeholder="Facoltativa" />
      </label>
      <label className="flex flex-col gap-1 text-xs font-medium">
        Argomento
        <textarea
          id="prenota-argomento"
          rows={3}
          maxLength={1000}
          value={dati.argomento}
          onChange={scrivi("argomento")}
          placeholder="Di cosa vuoi parlare (facoltativo)"
          className="min-h-16 resize-y rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30"
        />
      </label>
      {/* lo vede e lo riempie solo un programma: chi lo riempie non prenota */}
      <input type="text" name="sito" tabIndex={-1} autoComplete="off" aria-hidden="true" value={dati.sito} onChange={scrivi("sito")} className="absolute -left-[9999px] size-px opacity-0" />
      {(regola || esito) && (
        <span id="prenota-esito" className={cn("text-xs", regola || (esito && esito !== "Prenoto…") ? "text-destructive" : "text-muted-foreground")}>
          {regola || esito}
        </span>
      )}
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" size="sm" onClick={onChiudi}>
          Annulla
        </Button>
        <Button type="submit" size="sm" id="prenota-invia" disabled={inCorso || !!regola || !completo}>
          Prenota
        </Button>
      </div>
    </form>
  );
}
