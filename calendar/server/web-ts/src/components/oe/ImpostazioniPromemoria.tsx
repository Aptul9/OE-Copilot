"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import { dovePagina } from "@/lib/oe/logic";
import type { ContenutoPromemoria, ImpostazioniPromemoria as Impostazioni, PromemoriaRisposta } from "@/lib/oe/types";

const CONTENUTI: { valore: ContenutoPromemoria; testo: string }[] = [
  { valore: "completo", testo: "Completo" },
  { valore: "minimo", testo: "Solo ora e azienda" },
];

// Gli orari più usati, a caselle. 0 è "all'inizio" (nell'ultimo minuto prima). Gli altri valori si scrivono nel campo sotto.
const ORARI: { minuti: number; testo: string }[] = [
  { minuti: 60, testo: "1 ora prima" },
  { minuti: 30, testo: "30 minuti prima" },
  { minuti: 15, testo: "15 minuti prima" },
  { minuti: 10, testo: "10 minuti prima" },
  { minuti: 5, testo: "5 minuti prima" },
  { minuti: 0, testo: "All'inizio" },
];
// I nomi che il server accetta (SUONI in lib/core/promemoria.ts): il suono della notifica dell'app per Windows
const SUONI: [string, string][] = [
  ["Default", "Predefinito"],
  ["Reminder", "Promemoria"],
  ["Mail", "Posta"],
  ["IM", "Messaggio"],
  ["SMS", "SMS"],
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n): [string, string] => [n === 1 ? "Alarm" : `Alarm${n}`, `Allarme ${n}`]),
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n): [string, string] => [n === 1 ? "Call" : `Call${n}`, `Chiamata ${n}`]),
];

const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));
/** AAAA-MM-GGTHH:MM:SS (ora di Roma) come "12/10 15:20". */
const quando = (iso: string) => `${iso.slice(8, 10)}/${iso.slice(5, 7)} ${iso.slice(11, 16)}`;
const conNotifiche = () => "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;

type Dove = ReturnType<typeof dovePagina>;

/**
 * "Promemoria": quando avvisare, dove, cosa scrivere e il suono, in un posto solo. Si salva tutto sul server: l'app per Windows rilegge
 * le impostazioni a ogni giro (un minuto), il server a ogni suo giro. Nell'app per Windows si vedono notifica, avviso a comparsa e suono;
 * nell'app per Android niente (le notifiche ci sono sempre). Le notifiche del browser solo in un browser, e solo se funzionano: chiavi
 * VAPID sul server e un browser che le ha.
 */
export function ImpostazioniPromemoria() {
  const [dati, setDati] = useState<PromemoriaRisposta | null>(null);
  const [qui, setQui] = useState<Dove | null>(null);
  const [errore, setErrore] = useState("");
  const [altri, setAltri] = useState("");
  const [occupato, setOccupato] = useState(false);
  // questo browser è iscritto alle notifiche? null se non si sa o se il browser non le ha
  const [questo, setQuesto] = useState<boolean | null>(null);
  const [esitoPush, setEsitoPush] = useState("");

  const carica = useCallback(async () => {
    try {
      const r = await api("get_promemoria");
      setDati(r);
      setAltri(r.promemoria.minuti.filter((m) => !ORARI.some((o) => o.minuti === m)).join(", "));
      setErrore("");
    } catch (e) {
      setErrore(messaggio(e));
    }
  }, []);

  useEffect(() => {
    setQui(dovePagina(window, navigator.userAgent));
    void carica();
  }, [carica]);

  useEffect(() => {
    if (qui !== "browser" || !conNotifiche()) return;
    navigator.serviceWorker
      .getRegistration()
      .then((reg) => reg?.pushManager.getSubscription())
      .then((sub) => setQuesto(!!sub))
      .catch(() => setQuesto(null));
  }, [qui]);

  const salva = async (parziale: Partial<Impostazioni>) => {
    setOccupato(true);
    setErrore("");
    try {
      const r = await api("set_promemoria", parziale);
      if (r.ok) await carica();
      else setErrore(r.error ?? "Non salvato.");
    } catch (e) {
      setErrore(messaggio(e));
    } finally {
      setOccupato(false);
    }
  };

  // le caselle e il campo sono la stessa lista: ognuno cambia la sua parte e il resto resta com'è
  const salvaMinuti = (lista: number[]) => {
    if (lista.join(",") !== dati?.promemoria.minuti.join(",")) void salva({ minuti: lista });
  };
  const cambiaOrario = (minuti: number, acceso: boolean) => {
    const scelti = new Set(dati?.promemoria.minuti);
    if (acceso) scelti.add(minuti);
    else scelti.delete(minuti);
    salvaMinuti([...scelti].sort((a, b) => a - b));
  };
  const salvaAltri = () => {
    const valori = altri.split(/[\s,;]+/).filter(Boolean).map(Number);
    if (valori.some((v) => !Number.isInteger(v))) return setErrore("Solo numeri interi, separati da virgola.");
    const aCaselle = (dati?.promemoria.minuti ?? []).filter((m) => ORARI.some((o) => o.minuti === m));
    salvaMinuti([...new Set([...aCaselle, ...valori])].sort((a, b) => a - b));
  };

  // Iscrive questo browser: il permesso, il service worker (/sw.js), l'iscrizione al servizio di notifiche con la chiave pubblica del server,
  // e l'iscrizione si manda al server, che la tiene in gateway.db
  const attivaQuesto = async (chiave: string) => {
    setEsitoPush("");
    setOccupato(true);
    try {
      if ((await Notification.requestPermission()) !== "granted") throw new Error("permesso negato dal browser");
      await navigator.serviceWorker.register("/sw.js");
      const reg = await navigator.serviceWorker.ready;
      // un'iscrizione fatta con un'altra chiave del server (le chiavi VAPID cambiate) farebbe fallire subscribe(): si rifà da capo
      await (await reg.pushManager.getSubscription())?.unsubscribe();
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: chiave });
      const r = await api("iscrivi_push", sub.toJSON());
      if (!r.ok) throw new Error(r.error);
      setQuesto(true);
      await carica();
    } catch (e) {
      setEsitoPush(`Non riuscito: ${messaggio(e)}`);
    } finally {
      setOccupato(false);
    }
  };

  const disattivaQuesto = async () => {
    setEsitoPush("");
    setOccupato(true);
    try {
      const sub = await (await navigator.serviceWorker.getRegistration())?.pushManager.getSubscription();
      if (sub) {
        await api("disiscrivi_push", sub.endpoint);
        await sub.unsubscribe();
      }
      setQuesto(false);
      await carica();
    } catch (e) {
      setEsitoPush(`Non riuscito: ${messaggio(e)}`);
    } finally {
      setOccupato(false);
    }
  };

  const prova = async () => {
    setEsitoPush("Invio…");
    try {
      const r = await api("prova_promemoria", "push");
      setEsitoPush(r.ok ? "Inviata." : `Non riuscita: ${r.error}`);
    } catch (e) {
      setEsitoPush(`Non riuscita: ${messaggio(e)}`);
    }
  };

  if (!dati || !qui) {
    return (
      <section id="cfg-promemoria">
        <h3 className="mb-2 font-semibold">Promemoria</h3>
        <p className="text-xs text-muted-foreground">{errore || "Carico…"}</p>
      </section>
    );
  }
  const p = dati.promemoria;
  const push = dati.canali.find((c) => c.nome === "push");
  const chiave = push?.chiavi ? push.chiave_pubblica : null;
  // un interruttore che non può funzionare non si mostra
  const pushQui = qui === "browser" && !!chiave && conNotifiche();

  return (
    <section id="cfg-promemoria">
      <h3 className="mb-2 font-semibold">Promemoria</h3>
      <div className="flex flex-col gap-3">
        <fieldset className="grid grid-cols-2 gap-x-4 gap-y-1.5" id="cfg-promemoria-orari">
          <legend className="mb-1 text-xs font-medium">Quando</legend>
          {ORARI.map((o) => (
            <label key={o.minuti} className="flex items-center gap-2">
              <input
                id={`cfg-promemoria-orario-${o.minuti}`}
                type="checkbox"
                className="size-4 accent-primary"
                checked={p.minuti.includes(o.minuti)}
                disabled={occupato}
                onChange={(e) => cambiaOrario(o.minuti, e.target.checked)}
              />
              {o.testo}
            </label>
          ))}
          <label className="col-span-2 mt-1 flex flex-col gap-1 text-xs font-medium">
            Altri minuti prima
            <Input
              id="cfg-promemoria-minuti"
              className="w-48"
              inputMode="numeric"
              placeholder="20, 45"
              value={altri}
              disabled={occupato}
              onChange={(e) => setAltri(e.target.value)}
              onBlur={salvaAltri}
              onKeyDown={(e) => e.key === "Enter" && salvaAltri()}
            />
          </label>
        </fieldset>

        {qui === "windows" && (
          <div className="flex flex-col gap-2" id="cfg-promemoria-dove">
            <span className="text-xs font-medium">Dove</span>
            <label className="flex items-center gap-2">
              <Switch id="cfg-promemoria-notifica-windows" checked={p.notifica_windows} disabled={occupato} onCheckedChange={(v) => void salva({ notifica_windows: v })} />
              Notifica di Windows
            </label>
            <label className="flex items-center gap-2">
              <Switch id="cfg-promemoria-comparsa" checked={p.comparsa} disabled={occupato} onCheckedChange={(v) => void salva({ comparsa: v })} />
              Avviso a comparsa
            </label>
          </div>
        )}
        {pushQui && chiave && (
          <div className="flex flex-col gap-1" id="cfg-promemoria-dove">
            <span className="text-xs font-medium">Dove</span>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <label className="flex items-center gap-2">
                <Switch id="cfg-promemoria-push" checked={!!questo} disabled={occupato} onCheckedChange={(v) => void (v ? attivaQuesto(chiave) : disattivaQuesto())} />
                Notifiche del browser
              </label>
              {questo && (
                <Button id="btn-promemoria-prova" type="button" size="xs" variant="outline" onClick={() => void prova()}>
                  Prova
                </Button>
              )}
            </div>
            {esitoPush && <span className="text-xs text-muted-foreground">{esitoPush}</span>}
          </div>
        )}

        <label className="flex flex-col gap-1 text-xs font-medium">
          Contenuto
          <Select value={p.contenuto} disabled={occupato} onValueChange={(v) => void salva({ contenuto: v as ContenutoPromemoria })}>
            <SelectTrigger id="cfg-promemoria-contenuto" className="w-64">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {CONTENUTI.map((c) => (
                <SelectItem key={c.valore} value={c.valore}>
                  {c.testo}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>

        {qui === "windows" && (
          <label className="flex flex-col gap-1 text-xs font-medium">
            Suono
            <Select value={p.suono} disabled={occupato} onValueChange={(v) => void salva({ suono: v })}>
              <SelectTrigger id="cfg-promemoria-suono" className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {SUONI.map(([valore, testo]) => (
                  <SelectItem key={valore} value={valore}>
                    {testo}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
        )}

        {pushQui && dati.ultimo_errore && (
          <p id="promemoria-stato" className="text-xs text-muted-foreground">
            Ultimo errore: {dati.ultimo_errore}
            {dati.ultimo_errore_il && ` (${quando(dati.ultimo_errore_il)})`}
          </p>
        )}
        {errore && (
          <p id="promemoria-errore" role="alert" className="text-xs text-destructive">
            {errore}
          </p>
        )}
      </div>
    </section>
  );
}
