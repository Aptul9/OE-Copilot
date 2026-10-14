"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/oe/api";
import type { ContenutoPromemoria, ImpostazioniPromemoria as Impostazioni, PromemoriaRisposta } from "@/lib/oe/types";

const NOMI_CANALI: Record<string, string> = { telegram: "Telegram", push: "Notifiche del browser" };
const CONTENUTI: { valore: ContenutoPromemoria; testo: string }[] = [
  { valore: "completo", testo: "Completo: titolo, luogo e link" },
  { valore: "minimo", testo: "Minimo: solo ora e azienda" },
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
// I nomi che il server accetta (SUONI in lib/core/promemoria.ts): li usa solo l'app per Windows, come suono della notifica
const SUONI: [string, string][] = [
  ["Default", "Predefinito di Windows"],
  ["Reminder", "Promemoria"],
  ["Mail", "Posta"],
  ["IM", "Messaggio"],
  ["SMS", "SMS"],
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n): [string, string] => [n === 1 ? "Alarm" : `Alarm${n}`, `Allarme ${n}`]),
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n): [string, string] => [n === 1 ? "Call" : `Call${n}`, `Chiamata ${n}`]),
];

const messaggio = (e: unknown) => (e instanceof Error ? e.message : String(e));
/** AAAA-MM-GGTHH:MM:SS (ora di Roma) come "12/10 15:20:00". */
const quando = (iso: string) => `${iso.slice(8, 10)}/${iso.slice(5, 7)} ${iso.slice(11, 19)}`;
const nelBrowser = () => typeof window !== "undefined" && "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;

/**
 * "Promemoria": il server avvisa prima delle riunioni su un canale (Telegram, notifiche del browser), anche a pagina chiusa. Le chiavi dei
 * canali stanno nell'ambiente del server, qui si vede solo se ci sono. Ogni modifica si salva subito e il server la rilegge al giro dopo.
 */
export function ImpostazioniPromemoria() {
  const [dati, setDati] = useState<PromemoriaRisposta | null>(null);
  const [errore, setErrore] = useState("");
  const [altri, setAltri] = useState("");
  const [occupato, setOccupato] = useState(false);
  const [prove, setProve] = useState<Record<string, string>>({});
  // questo browser è iscritto alle notifiche? null se non si sa o se il browser non le ha
  const [questo, setQuesto] = useState<boolean | null>(null);

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
    void carica();
  }, [carica]);

  useEffect(() => {
    if (!nelBrowser()) return;
    navigator.serviceWorker
      .getRegistration()
      .then((reg) => reg?.pushManager.getSubscription())
      .then((sub) => setQuesto(!!sub))
      .catch(() => setQuesto(null));
  }, []);

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
    if (valori.some((v) => !Number.isInteger(v))) return setErrore("Scrivi numeri interi separati da virgola, per esempio 20, 45.");
    const aCaselle = (dati?.promemoria.minuti ?? []).filter((m) => ORARI.some((o) => o.minuti === m));
    salvaMinuti([...new Set([...aCaselle, ...valori])].sort((a, b) => a - b));
  };

  const dichiara = (nome: string, testo: string) => setProve((x) => ({ ...x, [nome]: testo }));

  const prova = async (nome: string) => {
    dichiara(nome, "Invio…");
    try {
      const r = await api("prova_promemoria", nome);
      dichiara(nome, r.ok ? "Prova inviata." : `Non riuscita: ${r.error}`);
    } catch (e) {
      dichiara(nome, `Non riuscita: ${messaggio(e)}`);
    }
  };

  // Iscrive questo browser: il permesso, il service worker (/sw.js), l'iscrizione al servizio di notifiche con la chiave pubblica del server,
  // e l'iscrizione si manda al server, che la tiene in gateway.db
  const attivaQuesto = async (chiave: string) => {
    dichiara("push", "Attivo…");
    try {
      if (!nelBrowser()) throw new Error("questo browser non ha le notifiche");
      if ((await Notification.requestPermission()) !== "granted") throw new Error("il browser non ha dato il permesso (si cambia dalle impostazioni del sito)");
      await navigator.serviceWorker.register("/sw.js");
      const reg = await navigator.serviceWorker.ready;
      // un'iscrizione fatta con un'altra chiave del server (le chiavi VAPID cambiate) farebbe fallire subscribe(): si rifà da capo
      await (await reg.pushManager.getSubscription())?.unsubscribe();
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: chiave });
      const r = await api("iscrivi_push", sub.toJSON());
      if (!r.ok) throw new Error(r.error);
      setQuesto(true);
      dichiara("push", "Notifiche attive su questo browser.");
      await carica();
    } catch (e) {
      dichiara("push", `Non riuscito: ${messaggio(e)}`);
    }
  };

  const disattivaQuesto = async () => {
    dichiara("push", "Disattivo…");
    try {
      const sub = await (await navigator.serviceWorker.getRegistration())?.pushManager.getSubscription();
      if (sub) {
        await api("disiscrivi_push", sub.endpoint);
        await sub.unsubscribe();
      }
      setQuesto(false);
      dichiara("push", "Notifiche tolte da questo browser.");
      await carica();
    } catch (e) {
      dichiara("push", `Non riuscito: ${messaggio(e)}`);
    }
  };

  if (!dati) {
    return (
      <section id="cfg-promemoria">
        <h3 className="mb-2 font-semibold">Promemoria</h3>
        <p className="text-xs text-muted-foreground">{errore || "Carico…"}</p>
      </section>
    );
  }
  const p = dati.promemoria;
  const cambiaCanale = (nome: string, acceso: boolean) => {
    const scelti = new Set(p.canali);
    if (acceso) scelti.add(nome);
    else scelti.delete(nome);
    void salva({ canali: [...scelti] });
  };

  return (
    <section id="cfg-promemoria">
      <h3 className="mb-2 font-semibold">Promemoria</h3>
      <div className="flex flex-col gap-3">
        <label className="flex items-center gap-2">
          <Switch id="cfg-promemoria-attivo" checked={p.attivo} disabled={occupato} onCheckedChange={(v) => void salva({ attivo: v })} /> Avvisami prima delle riunioni
        </label>
        <p className="-mt-1.5 text-xs text-muted-foreground">Li manda il server, anche a pagina chiusa. Di norma è spento: si accende con un canale configurato sul server.</p>

        <fieldset className="flex flex-col gap-1.5" id="cfg-promemoria-orari">
          <legend className="mb-1 text-xs font-medium">Quando avvisare</legend>
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
          <label className="mt-1 flex flex-col gap-1 text-xs font-medium">
            Altri minuti prima (separati da virgola, per esempio 20, 45)
            <Input
              id="cfg-promemoria-minuti"
              className="w-48"
              inputMode="numeric"
              value={altri}
              disabled={occupato}
              onChange={(e) => setAltri(e.target.value)}
              onBlur={salvaAltri}
              onKeyDown={(e) => e.key === "Enter" && salvaAltri()}
            />
          </label>
        </fieldset>

        <label className="flex flex-col gap-1 text-xs font-medium">
          Suono della notifica di Windows (solo l&apos;app per Windows)
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
        <p className="-mt-1.5 text-xs text-muted-foreground">L&apos;app rilegge orari e suono ogni 30 minuti, o quando si riavvia. Gli stessi orari valgono per i canali sotto.</p>

        <div className="flex flex-col gap-2">
          <span className="text-xs font-medium">Dove</span>
          {dati.canali.map((c) => {
            const scelto = p.canali.includes(c.nome);
            // per le notifiche del browser "configurato" vuol dire pronte: chiavi sul server e almeno un browser iscritto
            const senzaChiavi = c.nome === "push" ? c.chiavi === false : !c.configurato;
            return (
              <div key={c.nome} className="flex flex-col gap-1">
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                  <label className="flex items-center gap-2">
                    <Switch
                      id={`cfg-promemoria-canale-${c.nome}`}
                      checked={scelto}
                      disabled={occupato || (!c.configurato && !scelto)}
                      onCheckedChange={(v) => cambiaCanale(c.nome, v)}
                    />
                    {NOMI_CANALI[c.nome] ?? c.nome}
                  </label>
                  {senzaChiavi && <span className="text-xs text-muted-foreground">non configurato sul server</span>}
                  {c.nome === "push" && !senzaChiavi && (
                    <span className="text-xs text-muted-foreground">{c.iscrizioni ? `${c.iscrizioni} browser iscritti` : "nessun browser iscritto"}</span>
                  )}
                  <Button id={`btn-promemoria-prova-${c.nome}`} type="button" size="xs" variant="outline" disabled={!c.configurato} onClick={() => void prova(c.nome)}>
                    Invia una prova
                  </Button>
                </div>
                {c.nome === "push" && !senzaChiavi && c.chiave_pubblica && (
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    {questo ? (
                      <Button id="btn-promemoria-push-disattiva" type="button" size="xs" variant="outline" onClick={() => void disattivaQuesto()}>
                        Togli da questo browser
                      </Button>
                    ) : (
                      <Button id="btn-promemoria-push-attiva" type="button" size="xs" variant="outline" disabled={!nelBrowser()} onClick={() => void attivaQuesto(c.chiave_pubblica as string)}>
                        Attiva su questo browser
                      </Button>
                    )}
                    {!nelBrowser() && <span className="text-xs text-muted-foreground">questo browser non ha le notifiche</span>}
                  </div>
                )}
                {prove[c.nome] && <span className="text-xs text-muted-foreground">{prove[c.nome]}</span>}
              </div>
            );
          })}
        </div>

        <label className="flex flex-col gap-1 text-xs font-medium">
          Cosa c&apos;è nel messaggio
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
        {p.contenuto === "completo" && <p className="-mt-1.5 text-xs text-muted-foreground">Con il messaggio completo il titolo delle riunioni passa dai server del canale scelto (cifrato da un capo all&apos;altro solo nelle notifiche del browser).</p>}

        <p id="promemoria-stato" className="text-xs text-muted-foreground">
          {dati.ultimo_giro ? `Ultimo controllo: ${quando(dati.ultimo_giro)}.` : "Nessun controllo da quando il server è partito."}
          {dati.ultimo_errore && ` Ultimo problema: ${dati.ultimo_errore}${dati.ultimo_errore_il ? ` (${quando(dati.ultimo_errore_il)})` : ""}.`}
        </p>
        {errore && (
          <p id="promemoria-errore" role="alert" className="text-xs text-destructive">
            {errore}
          </p>
        )}
      </div>
    </section>
  );
}
