"use client";

import { LogOutIcon } from "lucide-react";
import { useInfoApp } from "@/components/ServerApp";
import { Button, buttonVariants } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { Modo2fa } from "@/lib/config";
import { esci } from "@/lib/esci";
import { dataEvento, fmtRange } from "@/lib/oe/logic";
import type { TemaConfig } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
import { ImpostazioniCondivisione } from "./ImpostazioniCondivisione";
import { ImpostazioniPromemoria } from "./ImpostazioniPromemoria";
import { useOe } from "./OeProvider";

type Props = { aperto: boolean; onChiudi: () => void; accesso2fa?: Modo2fa };

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

/** Impostazioni: aspetto e ogni quanto si rilegge il calendario. */
export function DialogoImpostazioni({ aperto, onChiudi, accesso2fa = "off" }: Props) {
  const { tema, impostaTema, intervalloSync, impostaIntervallo, config, salvaConfig } = useOe();

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      {/* Larga e a due colonne, così sta sullo schermo senza scorrere; su uno schermo stretto torna a una colonna e scorre. */}
      <DialogContent id="modal-appsettings" className="max-h-[92dvh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Impostazioni</DialogTitle>
          <DialogDescription className="sr-only">Aspetto e sincronizzazione</DialogDescription>
        </DialogHeader>
        <div className="grid gap-x-10 gap-y-5 text-sm md:grid-cols-2">
          <div className="flex flex-col gap-5">
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
              <label className="mt-3 flex items-center gap-2">
                <Switch id="cfg-colori-dinamici" checked={!!config.colori_dinamici} onCheckedChange={(v) => void salvaConfig({ colori_dinamici: v })} /> Colori dinamici
              </label>
              <p className="mt-1.5 text-xs text-muted-foreground">Riunioni vicine di aziende diverse mai dello stesso colore.</p>
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

            <EventiEliminati />

            {accesso2fa !== "off" && (
              <section>
                <h3 className="mb-2 font-semibold">Sicurezza</h3>
                <p className="mb-2 text-xs text-muted-foreground">Codice dell&apos;app di autenticazione{accesso2fa === "passkey" ? " e passkey" : ""}.</p>
                <a id="cfg-sicurezza" href="/sicurezza" className={buttonVariants({ variant: "outline", size: "sm" })}>
                  Gestisci l&apos;accesso
                </a>
              </section>
            )}
            <ImpostazioniCondivisione />
          </div>
          <ImpostazioniPromemoria />
          <Account />
        </div>
        <DialogFooter>
          <Button onClick={onChiudi}>Fine</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** L'uscita dall'account e, dentro l'app per Windows e Android, il server: *Cambia server* porta alla pagina di avvio dell'app, dove si scrive l'indirizzo. */
function Account() {
  const app = useInfoApp();
  return (
    <section className="border-t pt-4 md:col-span-2">
      <h3 className="mb-2 font-semibold">Account</h3>
      <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
        {app && (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-muted-foreground">Server</span>
            <b id="cfg-server">{app.host}</b>
            <a id="cfg-server-cambia" href={app.cambiaServer} className={buttonVariants({ variant: "outline", size: "sm" })}>
              Cambia server
            </a>
          </div>
        )}
        <Button id="cfg-esci" variant="outline" size="sm" onClick={esci}>
          <LogOutIcon /> Esci dall&apos;account
        </Button>
      </div>
    </section>
  );
}

/** Gli eventi tolti dalla vista con Elimina, l'ultimo in cima: Ripristina li rimette. */
function EventiEliminati() {
  const { config, ripristinaEvento } = useOe();
  const tutti = config.eventi_eliminati ?? [];
  if (!tutti.length) return null;
  return (
    <section>
      <h3 className="mb-2 font-semibold">Eventi eliminati</h3>
      <ul id="cfg-eliminati" className="flex flex-col gap-2">
        {[...tutti].reverse().map((v) => (
          <li key={v.chiave} className="flex items-center gap-2">
            <div className="min-w-0 flex-1">
              <div className="truncate font-medium">{v.title}</div>
              <div className="truncate text-xs text-muted-foreground">
                {fmtRange(dataEvento(v.start, v.allDay), dataEvento(v.end || v.start, v.allDay), v.allDay)}
                {v.alias && ` · ${v.alias}`}
              </div>
            </div>
            <Button size="sm" variant="outline" onClick={() => void ripristinaEvento(v.chiave)}>
              Ripristina
            </Button>
          </li>
        ))}
      </ul>
    </section>
  );
}
