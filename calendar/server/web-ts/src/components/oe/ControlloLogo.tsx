"use client";

import { CameraIcon, ImageUpIcon, LinkIcon, RotateCwIcon, Trash2Icon, UploadIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { api } from "@/lib/oe/api";
import { dominioScritto, iconaDelSito, indirizzoDaTesto, iniziali, mappaLoghi } from "@/lib/oe/logic";
import type { VoceLogo } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { ImgLogo } from "./ImgLogo";
import { useOe } from "./OeProvider";

// Il logo di un nome mostrato, nella finestra Account: un cerchio con l'immagine al centro, o le iniziali nel colore del calendario. Toccare il
// cerchio apre "Logo di …", con il cerchio grande: si tocca, o ci si trascina sopra un file, per cambiare l'immagine; oppure la si prende da un
// sito o da un link. L'immagine la scarica il server (lib/core/loghi.ts), mai il browser.

const MAX_KB = 512;
const TIPI = "image/png,image/jpeg,image/gif,image/webp,image/x-icon,image/vnd.microsoft.icon,image/svg+xml,.ico";

/** Il logo di `nome` in un cerchio di `lato` px: l'immagine su fondo bianco, o le iniziali nel colore. */
export function CerchioLogo({ nome, colore, src, lato }: { nome: string; colore: string; src?: string; lato: number }) {
  const { pers } = useOe();
  const sigla = (
    <span className="grid size-full place-items-center font-extrabold leading-none text-white" style={{ background: colore, fontSize: Math.max(9, Math.round(lato * 0.36)) }}>
      {pers.interfaccia.sigle?.[nome] || iniziali(nome)}
    </span>
  );
  return (
    <span className="grid flex-none place-items-center overflow-hidden rounded-full bg-white ring-1 ring-black/10" style={{ width: lato, height: lato }}>
      {src ? <ImgLogo key={src} src={src} className="size-full object-contain p-[16%]" alternativa={sigla} /> : sigla}
    </span>
  );
}

type PropsLogo = { nome: string; colore: string; voce?: VoceLogo; onCambiato: () => Promise<void> };

/**
 * Il cerchio che si tocca per cambiare il logo. Al passaggio del mouse mostra una macchina fotografica; `segno` la tiene sempre in un angolo,
 * per chi usa il tocco.
 */
export function AvatarLogo({ lato, segno = false, ...logo }: PropsLogo & { lato: number; segno?: boolean }) {
  const { pers } = useOe();
  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  const [aperto, setAperto] = useState(false);
  const angolo = Math.max(18, Math.round(lato * 0.4));
  return (
    <>
      <button
        type="button"
        data-logo-apri={logo.nome}
        aria-label={`Cambia il logo di ${logo.nome}`}
        title={`Cambia il logo di ${logo.nome}`}
        className="group relative flex-none rounded-full outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
        onClick={() => setAperto(true)}
      >
        <CerchioLogo nome={logo.nome} colore={logo.colore} src={loghi[logo.nome]} lato={lato} />
        <span aria-hidden className="absolute inset-0 grid place-items-center rounded-full bg-black/50 text-white opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100">
          <CameraIcon style={{ width: Math.round(lato * 0.45), height: Math.round(lato * 0.45) }} />
        </span>
        {segno && (
          <span aria-hidden className="absolute -bottom-1 -right-1 grid place-items-center rounded-full bg-primary text-primary-foreground ring-2 ring-popover" style={{ width: angolo, height: angolo }}>
            <CameraIcon style={{ width: Math.round(angolo * 0.55), height: Math.round(angolo * 0.55) }} />
          </span>
        )}
      </button>
      {aperto && <DialogoLogo {...logo} onChiudi={() => setAperto(false)} />}
    </>
  );
}

const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));

type Occupato = "anteprima" | "salva" | "rimuovi" | "aggiorna" | "file" | null;

/** "Logo di …": il cerchio grande al centro. Si apre già montato e si smonta alla chiusura, così ogni volta riparte pulito. */
function DialogoLogo({ nome, colore, voce, onCambiato, onChiudi }: PropsLogo & { onChiudi: () => void }) {
  const { pers, ricaricaPers } = useOe();
  const loghi = useMemo(() => mappaLoghi(pers.interfaccia.loghi), [pers]);
  const attuale = loghi[nome];
  const [url, setUrl] = useState(voce?.url ?? "");
  const [conLink, setConLink] = useState(false);
  const [anteprima, setAnteprima] = useState<{ src: string; testo: string; indirizzo: string } | null>(null);
  const [sopra, setSopra] = useState(false);
  const [errore, setErrore] = useState("");
  const [esito, setEsito] = useState("");
  const [occupato, setOccupato] = useState<Occupato>(null);
  const file = useRef<HTMLInputElement>(null);

  // una scelta nuova (salvata da qui, o fatta da un altro dispositivo) riporta il suo indirizzo nel campo
  useEffect(() => setUrl(voce?.url ?? ""), [voce?.url]);

  const dopoModifica = async (testo: string) => {
    setAnteprima(null);
    setConLink(false);
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

  // un dominio da solo (msc.com) vale l'icona del sito
  const vediAnteprima = () =>
    fai("anteprima", async () => {
      const dominio = dominioScritto(url);
      const indirizzo = dominio ? iconaDelSito(dominio) : indirizzoDaTesto(url);
      setAnteprima(null);
      if (!indirizzo) throw new Error("Scrivi il sito o l'indirizzo dell'immagine.");
      if (dominio) setUrl(indirizzo);
      setEsito("Verifico l'indirizzo…");
      const r = await api("preview_logo", indirizzo);
      setEsito("");
      if ((!r.ok || !r.anteprima) && dominio) throw new Error(`${dominio} non ha un'icona in /favicon.ico: scrivi l'indirizzo dell'immagine.`);
      if (!r.ok || !r.anteprima) throw new Error(r.error || "Immagine non leggibile.");
      setAnteprima({ src: r.anteprima, testo: `${r.tipo} · ${Math.max(1, Math.round((r.byte ?? 0) / 1024))} KB`, indirizzo });
    });

  const usa = () =>
    fai("salva", async () => {
      if (!anteprima) return;
      setEsito("Scarico l'immagine…");
      const r = await api("set_logo", nome, anteprima.indirizzo);
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
      if (!f.type.startsWith("image/") && !/\.ico$/i.test(f.name)) throw new Error("Non è un'immagine.");
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

  const trascina = (e: DragEvent) => {
    e.preventDefault();
    if (occupato === null) setSopra(true);
  };
  const lascia = (e: DragEvent) => {
    e.preventDefault();
    setSopra(false);
    if (occupato === null) void caricaFile(e.dataTransfer.files[0]);
  };

  const origine = voce
    ? voce.tipo === "file"
      ? "Immagine caricata da un file."
      : `Scaricato il ${new Date(voce.aggiornato).toLocaleString("it-IT", { dateStyle: "short", timeStyle: "short" })}.`
    : attuale
      ? nome === "Eventi personali" && attuale === "/logos/personale.png"
        ? "Logo predefinito."
        : "Da personalizzazione.json."
      : "Nessun logo: si vedono le iniziali.";
  const libero = occupato === null;

  return (
    <Dialog open onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-logo" data-logo-pannello={nome} className="gap-3 sm:max-w-md" onOpenAutoFocus={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>Logo di {nome}</DialogTitle>
          <DialogDescription>Si vede nella barra accanto a {nome} e negli account.</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col items-center gap-2 py-2">
          <button
            type="button"
            data-logo="cerchio"
            aria-label={`Scegli un'immagine per ${nome}`}
            title="Scegli un'immagine, o trascinala qui"
            disabled={!libero}
            className="group relative rounded-full outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-wait"
            onClick={() => file.current?.click()}
            onDragEnter={trascina}
            onDragOver={trascina}
            onDragLeave={() => setSopra(false)}
            onDrop={lascia}
          >
            <CerchioLogo nome={nome} colore={colore} src={anteprima?.src ?? attuale} lato={136} />
            <span
              aria-hidden
              className={cn(
                "absolute inset-0 flex flex-col items-center justify-center gap-1 rounded-full bg-black/55 text-xs font-medium text-white transition-opacity",
                sopra || !libero ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100",
              )}
            >
              {!libero ? <Spinner className="size-6" /> : <ImageUpIcon className="size-6" />}
              {!libero ? "" : sopra ? "Lascia qui" : "Cambia immagine"}
            </span>
            <span aria-hidden className="absolute bottom-0 right-0 grid size-10 place-items-center rounded-full bg-primary text-primary-foreground ring-4 ring-popover">
              <CameraIcon className="size-5" />
            </span>
            {anteprima && (
              <span className="absolute -top-2 left-1/2 -translate-x-1/2 rounded-full bg-primary px-2 py-0.5 text-[11px] font-semibold text-primary-foreground">Anteprima</span>
            )}
          </button>
          <p data-logo="origine" className="text-center text-xs text-muted-foreground">
            {anteprima ? anteprima.testo : origine}
          </p>
        </div>

        {anteprima ? (
          <div className="flex justify-center gap-2">
            <Button data-logo="usa" disabled={!libero} onClick={() => void usa()}>
              Usa questo logo
            </Button>
            <Button variant="outline" disabled={!libero} onClick={() => setAnteprima(null)}>
              Annulla
            </Button>
          </div>
        ) : (
          <div className="flex flex-wrap justify-center gap-2">
            <Button data-logo="file-scegli" size="sm" disabled={!libero} onClick={() => file.current?.click()}>
              <UploadIcon /> Carica un&apos;immagine
            </Button>
            <Button data-logo="da-link" variant={conLink ? "secondary" : "outline"} size="sm" aria-expanded={conLink} disabled={!libero} onClick={() => setConLink((v) => !v)}>
              <LinkIcon /> Da un sito o link
            </Button>
            {voce?.tipo === "url" && (
              <Button data-logo="aggiorna" variant="ghost" size="sm" disabled={!libero} onClick={() => void aggiorna()} title="Scarica di nuovo l'immagine">
                <RotateCwIcon /> Aggiorna
              </Button>
            )}
            {voce && (
              <Button data-logo="rimuovi" variant="ghost" size="sm" className="text-destructive hover:text-destructive" disabled={!libero} onClick={() => void rimuovi()}>
                <Trash2Icon /> Rimuovi
              </Button>
            )}
          </div>
        )}

        {conLink && !anteprima && (
          <label className="flex flex-col gap-1 rounded-lg border bg-muted/30 p-3 text-xs font-medium">
            Il sito dell&apos;azienda o l&apos;indirizzo dell&apos;immagine
            <span className="flex gap-2">
              <Input
                data-logo="url"
                autoFocus
                className="h-8 min-w-0 flex-1 font-normal"
                value={url}
                placeholder="msc.com o https://…/logo.png"
                spellCheck={false}
                autoComplete="off"
                inputMode="url"
                onChange={(e) => {
                  setUrl(e.target.value);
                  setErrore("");
                  setEsito("");
                }}
                onKeyDown={(e) => e.key === "Enter" && libero && url.trim() && void vediAnteprima()}
              />
              <Button data-logo="anteprima-vedi" variant="outline" size="sm" className="h-8" disabled={!libero || !url.trim()} onClick={() => void vediAnteprima()}>
                Anteprima
              </Button>
            </span>
            <span className="font-normal text-muted-foreground">Con solo il sito prende la sua icona.</span>
          </label>
        )}

        {(errore || esito) && (
          <p data-logo={errore ? "errore" : "esito"} role={errore ? "alert" : "status"} className={cn("text-center text-xs", errore ? "text-destructive" : "text-muted-foreground")}>
            {errore || esito}
          </p>
        )}
        <p className="text-center text-xs text-muted-foreground">PNG, JPEG, GIF, WebP, ICO o SVG, massimo {MAX_KB} KB. Un file si può anche trascinare sul cerchio.</p>
        <input
          ref={file}
          data-logo="file"
          type="file"
          accept={TIPI}
          className="sr-only"
          tabIndex={-1}
          onChange={(e) => {
            void caricaFile(e.target.files?.[0]);
            e.target.value = "";
          }}
        />
      </DialogContent>
    </Dialog>
  );
}
