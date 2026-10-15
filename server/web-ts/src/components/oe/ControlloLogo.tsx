"use client";

import { RotateCwIcon, Trash2Icon, UploadIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/oe/api";
import { dominioScritto, iconaDelSito, indirizzoDaTesto, iniziali, mappaLoghi } from "@/lib/oe/logic";
import type { VoceLogo } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { ImgLogo } from "./ImgLogo";
import { useOe } from "./OeProvider";

// Il logo di un'azienda dalla finestra Calendari: il pulsante accanto al nome (mostra quello di adesso) e il pannello con l'indirizzo, l'anteprima,
// "Usa questo logo", "Rimuovi", "Aggiorna" e il file da caricare. L'immagine la scarica il server (lib/core/loghi.ts), mai il browser.

const MAX_KB = 512;

/** Il logo di `nome` come la barra lo mostra, o le sue iniziali nel colore dell'azienda. */
function Miniatura({ nome, colore, src, className }: { nome: string; colore: string; src?: string; className?: string }) {
  const { pers } = useOe();
  const sigle = pers.interfaccia.sigle ?? {};
  const sigla = (
    <span className="grid size-full place-items-center text-[9.5px] font-extrabold leading-none text-white" style={{ background: colore }}>
      {sigle[nome] || iniziali(nome)}
    </span>
  );
  return (
    <span className={cn("grid size-[22px] flex-none place-items-center overflow-hidden rounded-md bg-white ring-1 ring-black/10", className)}>
      {src ? <ImgLogo key={src} src={src} className="size-full object-contain p-[2px]" alternativa={sigla} /> : sigla}
    </span>
  );
}

/** Il pulsante accanto al nome dell'azienda: il logo di adesso e la parola Logo. */
export function BottoneLogo({ nome, colore, aperto, onClick }: { nome: string; colore: string; aperto: boolean; onClick: () => void }) {
  const { pers } = useOe();
  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  return (
    <button
      type="button"
      data-logo-apri={nome}
      aria-expanded={aperto}
      aria-label={`Logo di ${nome}`}
      title={`Cambia il logo di ${nome}`}
      className={cn("flex flex-none items-center gap-1.5 rounded-md border px-1 py-0.5 text-[11px] font-medium text-muted-foreground hover:bg-background hover:text-foreground", aperto && "bg-background text-foreground")}
      onClick={onClick}
    >
      <Miniatura nome={nome} colore={colore} src={loghi[nome]} />
      Logo
    </button>
  );
}

const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));

type Occupato = "anteprima" | "salva" | "rimuovi" | "aggiorna" | "file" | null;

/** Il pannello sotto il nome: dove si sceglie il logo. `voce` è la scelta fatta da qui, se c'è; `onCambiato` rilegge l'elenco delle scelte. */
export function PannelloLogo({ nome, colore, voce, onCambiato }: { nome: string; colore: string; voce?: VoceLogo; onCambiato: () => Promise<void> }) {
  const { pers, ricaricaPers } = useOe();
  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  const attuale = loghi[nome];
  const [url, setUrl] = useState(voce?.url ?? "");
  const [anteprima, setAnteprima] = useState<{ src: string; testo: string } | null>(null);
  const [errore, setErrore] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState<Occupato>(null);
  const file = useRef<HTMLInputElement>(null);
  const dominio = dominioScritto(url);

  // una scelta nuova (salvata da qui, o fatta da un altro dispositivo) riporta il suo indirizzo nel campo
  useEffect(() => setUrl(voce?.url ?? ""), [voce?.url]);

  const dopoModifica = async (testo: string) => {
    setAnteprima(null);
    setEsito(testo);
    await ricaricaPers();
    await onCambiato();
  };

  const fai = async (cosa: Exclude<Occupato, null>, lavoro: () => Promise<void>) => {
    setErrore("");
    setEsito("");
    setOccupato(cosa);
    try {
      await lavoro();
    } catch (e) {
      setErrore(messaggio(e));
    } finally {
      setOccupato(null);
    }
  };

  const vediAnteprima = (indirizzo = url) =>
    fai("anteprima", async () => {
      const testo = indirizzoDaTesto(indirizzo);
      setAnteprima(null);
      if (!testo) throw new Error("Scrivi l'indirizzo dell'immagine.");
      setEsito("Verifico l'indirizzo…");
      const r = await api("preview_logo", testo);
      setEsito("");
      if (!r.ok || !r.anteprima) throw new Error(r.error || "Immagine non leggibile.");
      setAnteprima({ src: r.anteprima, testo: `${r.tipo} · ${Math.max(1, Math.round((r.byte ?? 0) / 1024))} KB` });
    });

  const usa = () =>
    fai("salva", async () => {
      const testo = indirizzoDaTesto(url);
      if (!testo) throw new Error("Scrivi l'indirizzo dell'immagine.");
      setEsito("Scarico l'immagine…");
      const r = await api("set_logo", nome, testo);
      if (!r.ok) throw new Error(r.error || "Logo non salvato.");
      await dopoModifica("Logo salvato.");
    });

  const rimuovi = () =>
    fai("rimuovi", async () => {
      const r = await api("remove_logo", nome);
      if (!r.ok) throw new Error(r.error || "Logo non rimosso.");
      setUrl("");
      await dopoModifica("Logo rimosso.");
    });

  const aggiorna = () =>
    fai("aggiorna", async () => {
      setEsito("Scarico l'immagine…");
      const r = await api("refresh_logo", nome);
      if (!r.ok) throw new Error(r.error || "Logo non aggiornato.");
      await dopoModifica("Logo aggiornato.");
    });

  const caricaFile = (f: File | undefined) =>
    fai("file", async () => {
      if (!f) return;
      if (f.size > MAX_KB * 1024) throw new Error(`Immagine troppo grande (massimo ${MAX_KB} KB)`);
      const base64 = await new Promise<string>((ok, no) => {
        const lettore = new FileReader();
        lettore.onload = () => ok(String(lettore.result).split(",")[1] ?? "");
        lettore.onerror = () => no(new Error("File non leggibile."));
        lettore.readAsDataURL(f);
      });
      const r = await api("upload_logo", nome, base64);
      if (!r.ok) throw new Error(r.error || "Logo non salvato.");
      await dopoModifica("Logo caricato.");
    });

  const origine = voce
    ? voce.tipo === "file"
      ? "File caricato."
      : `Scaricato il ${new Date(voce.aggiornato).toLocaleString("it-IT", { dateStyle: "short", timeStyle: "short" })}.`
    : attuale
      ? nome === "Eventi personali" && attuale === "/logos/personale.png"
        ? "Logo predefinito."
        : "Da personalizzazione.json."
      : "Nessun logo.";

  return (
    <div className="mb-2 ml-5 rounded-lg border bg-muted/30 p-3 text-sm" data-logo-pannello={nome}>
      <div className="flex items-center gap-3">
        <Miniatura nome={nome} colore={colore} src={attuale} className="size-10 rounded-lg" />
        <div className="min-w-0">
          <div className="truncate font-medium">Logo di {nome}</div>
          <div className="text-xs text-muted-foreground" data-logo="origine">
            {origine}
          </div>
        </div>
      </div>

      <label className="mt-3 flex flex-col gap-1 text-xs font-medium">
        Indirizzo dell&apos;immagine
        <span className="flex gap-2">
          <Input
            data-logo="url"
            className="h-8 min-w-0 flex-1 font-normal"
            value={url}
            placeholder="https://esempio.org/logo.png"
            spellCheck={false}
            autoComplete="off"
            inputMode="url"
            onChange={(e) => {
              setUrl(e.target.value);
              setAnteprima(null);
              setErrore("");
              setEsito("");
            }}
            onKeyDown={(e) => e.key === "Enter" && occupato === null && url.trim() && void vediAnteprima()}
          />
          <Button data-logo="anteprima-vedi" variant="outline" size="sm" className="h-8" disabled={occupato !== null || !url.trim()} onClick={() => void vediAnteprima()}>
            Anteprima
          </Button>
        </span>
      </label>

      {dominio && (
        <button
          type="button"
          data-logo="icona-sito"
          className="mt-1.5 text-xs text-primary underline-offset-2 hover:underline"
          onClick={() => {
            const indirizzo = iconaDelSito(dominio);
            setUrl(indirizzo);
            void vediAnteprima(indirizzo);
          }}
        >
          Usa l&apos;icona di {dominio}
        </button>
      )}

      {anteprima && (
        <div className="mt-2 flex items-center gap-3">
          {/* data: URL fatto dal server: il browser non contatta l'indirizzo */}
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img data-logo="anteprima" src={anteprima.src} alt={`Anteprima del logo di ${nome}`} className="size-16 rounded-md bg-white object-contain p-1 ring-1 ring-black/10" />
          <span className="text-xs text-muted-foreground">{anteprima.testo}</span>
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Button data-logo="usa" size="sm" disabled={occupato !== null || !url.trim()} onClick={() => void usa()}>
          Usa questo logo
        </Button>
        {voce && (
          <Button data-logo="rimuovi" variant="outline" size="sm" disabled={occupato !== null} onClick={() => void rimuovi()}>
            <Trash2Icon /> Rimuovi
          </Button>
        )}
        {voce?.tipo === "url" && (
          <Button data-logo="aggiorna" variant="outline" size="sm" disabled={occupato !== null} onClick={() => void aggiorna()} title="Scarica di nuovo l'immagine">
            <RotateCwIcon /> Aggiorna
          </Button>
        )}
        <Button data-logo="file-scegli" variant="ghost" size="sm" disabled={occupato !== null} onClick={() => file.current?.click()}>
          <UploadIcon /> Carica un file
        </Button>
        <input
          ref={file}
          data-logo="file"
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp,image/x-icon,image/vnd.microsoft.icon,image/svg+xml,.ico"
          className="sr-only"
          tabIndex={-1}
          onChange={(e) => {
            void caricaFile(e.target.files?.[0]);
            e.target.value = "";
          }}
        />
      </div>

      {(errore || esito) && (
        <p data-logo={errore ? "errore" : "esito"} role={errore ? "alert" : "status"} className={cn("mt-2 text-xs", errore ? "text-destructive" : "text-muted-foreground")}>
          {errore || esito}
        </p>
      )}
      <p className="mt-2 text-xs text-muted-foreground">PNG, JPEG, GIF, WebP, ICO o SVG, massimo {MAX_KB} KB. Con solo il dominio (esempio.org) prende l&apos;icona del sito.</p>
    </div>
  );
}
