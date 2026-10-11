"use client";

import { Button, buttonVariants } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { Modo2fa } from "@/lib/config";
import type { TemaConfig } from "@/lib/oe/types";
import { cn } from "@/lib/utils";
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
  const { tema, impostaTema, intervalloSync, impostaIntervallo } = useOe();

  return (
    <Dialog open={aperto} onOpenChange={(o) => !o && onChiudi()}>
      <DialogContent id="modal-appsettings" className="max-h-[86dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Impostazioni</DialogTitle>
          <DialogDescription className="sr-only">Aspetto e sincronizzazione</DialogDescription>
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

          {accesso2fa !== "off" && (
            <section>
              <h3 className="mb-2 font-semibold">Sicurezza</h3>
              <p className="mb-2 text-xs text-muted-foreground">
                Secondo fattore dell&apos;accesso: il codice di un&apos;app di autenticazione{accesso2fa === "passkey" ? " e le passkey" : ""}.
              </p>
              <a id="cfg-sicurezza" href="/sicurezza" className={buttonVariants({ variant: "outline", size: "sm" })}>
                Gestisci l&apos;accesso
              </a>
            </section>
          )}

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
