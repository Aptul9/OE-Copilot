"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { api } from "@/lib/oe/api";
import type { Integrazione, TemaConfig } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void };

const ETICHETTE_STATO: Record<string, { testo: string; classe: string; punto: string }> = {
  ok: { testo: "Collegato", classe: "text-emerald-600", punto: "bg-emerald-600" },
  problema: { testo: "Problema temporaneo", classe: "text-amber-600", punto: "bg-amber-500" },
  riautorizzare: { testo: "Da riautorizzare", classe: "text-red-600", punto: "bg-red-600" },
  assente: { testo: "Non configurato", classe: "text-red-600", punto: "bg-red-600" },
};
const TEMI: { valore: TemaConfig; testo: string }[] = [
  { valore: "auto", testo: "Automatico" },
  { valore: "light", testo: "Chiaro" },
  { valore: "dark", testo: "Scuro" },
];
const INTERVALLI = [
  { valore: 30, testo: "30 secondi" },
  { valore: 60, testo: "1 minuto" },
  { valore: 120, testo: "2 minuti" },
  { valore: 300, testo: "5 minuti" },
];

/** Impostazioni: aspetto, ogni quanto si rilegge il calendario, stato delle integrazioni. */
export function DialogoImpostazioni({ aperto, onChiudi }: Props) {
  const { tema, impostaTema, intervalloSync, impostaIntervallo } = useOe();
  const [voci, setVoci] = useState<Integrazione[] | null>(null);
  const [messaggio, setMessaggio] = useState("");
  const [verifica, setVerifica] = useState(false);

  const carica = useCallback(async (v: boolean) => {
    setVerifica(v);
    setVoci(null);
    setMessaggio("");
    try {
      const r = await api("get_integrazioni", v);
      setVoci(r.integrazioni ?? []);
    } catch (e) {
      setMessaggio(`Stato non leggibile: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, []);

  useEffect(() => {
    if (aperto) void carica(false);
  }, [aperto, carica]);

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-appsettings" className="max-h-[86dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Impostazioni</DialogTitle>
          <DialogDescription className="sr-only">Aspetto, sincronizzazione e integrazioni</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-5 text-sm">
          <section>
            <h3 className="mb-2 font-semibold">Aspetto</h3>
            <div id="cfg-theme" className="inline-flex rounded-lg border p-0.5">
              {TEMI.map((t) => (
                <button
                  key={t.valore}
                  data-theme={t.valore}
                  aria-pressed={tema === t.valore}
                  className={cn("rounded-md px-3 py-1 text-[13px]", tema === t.valore ? "is-active bg-accent font-semibold text-accent-foreground" : "text-muted-foreground hover:bg-muted")}
                  onClick={() => impostaTema(t.valore)}
                >
                  {t.testo}
                </button>
              ))}
            </div>
            <p className="mt-1.5 text-xs text-muted-foreground">&quot;Automatico&quot; segue il tema del dispositivo.</p>
          </section>

          <section>
            <h3 className="mb-2 font-semibold">Sincronizzazione</h3>
            <label className="flex flex-col gap-1 text-xs font-medium">
              Ricontrolla il calendario ogni
              <Select value={String(intervalloSync)} onValueChange={(v) => impostaIntervallo(Number(v))}>
                <SelectTrigger id="cfg-sync-interval" className="w-48">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {INTERVALLI.map((i) => (
                    <SelectItem key={i.valore} value={String(i.valore)}>
                      {i.testo}
                    </SelectItem>
                  ))}
                  {!INTERVALLI.some((i) => i.valore === intervalloSync) && <SelectItem value={String(intervalloSync)}>{intervalloSync} secondi</SelectItem>}
                </SelectContent>
              </Select>
            </label>
          </section>

          <section>
            <h3 className="mb-2 font-semibold">Integrazioni</h3>
            <div id="integrazioni-list" className="mb-2 flex flex-col gap-2">
              {!voci && !messaggio && <p className="text-xs text-muted-foreground">{verifica ? "Verifico…" : "Controllo…"}</p>}
              {messaggio && <p className="text-xs text-muted-foreground">{messaggio}</p>}
              {voci && voci.length === 0 && <p className="text-xs text-muted-foreground">Nessuna integrazione.</p>}
              {voci?.map((v, i) => {
                const e = ETICHETTE_STATO[v.stato] ?? ETICHETTE_STATO.problema;
                const dettagli = [];
                if (v.ultimo_ok) dettagli.push("ultimo aggiornamento " + v.ultimo_ok.replace("T", " ").slice(11, 16));
                if (v.stato === "ok" && v.scade) dettagli.push("token valido fino alle " + v.scade);
                return (
                  <div key={i} className="integr-row flex items-center gap-2.5 rounded-lg border px-3 py-2">
                    <span className={cn("size-2.5 flex-none rounded-full", e.punto)} />
                    <span className="flex min-w-0 flex-1 flex-col font-semibold">
                      {v.nome}
                      <small className="truncate text-[11px] font-normal text-muted-foreground">
                        {v.tipo} · {v.indirizzo}
                      </small>
                    </span>
                    <span className={cn("flex flex-col text-right font-semibold", e.classe)}>
                      {e.testo}
                      <small className="text-[11px] font-normal text-muted-foreground">{dettagli.join(" · ") || v.messaggio || ""}</small>
                    </span>
                  </div>
                );
              })}
            </div>
            <Button id="btn-verifica-integr" variant="outline" size="sm" onClick={() => void carica(true)}>
              Verifica adesso
            </Button>
          </section>

          <section>
            <h3 className="mb-1 font-semibold">Info</h3>
            <p className="text-xs text-muted-foreground">OE Copilot, la vista unica su calendari e timesheet di più aziende. I dati restano sul server.</p>
          </section>
        </div>
        <DialogFooter>
          <Button onClick={onChiudi}>Fine</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
