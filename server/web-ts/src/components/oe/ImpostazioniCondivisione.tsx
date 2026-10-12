"use client";

import { Share2Icon } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { RILETTURA_MINUTI } from "@/lib/condivisione-comune";
import { DialogoCondivisione } from "./DialogoCondivisione";

/** "Condivisione": nelle impostazioni c'è solo la descrizione e il pulsante; i link si gestiscono nella finestra che si apre da qui. */
export function ImpostazioniCondivisione() {
  const [aperto, setAperto] = useState(false);
  return (
    <section id="cfg-condivisione">
      <h3 className="mb-2 font-semibold">Condivisione</h3>
      <p className="mb-2 text-xs text-muted-foreground">
        Condividi il calendario con un altro OE Copilot: chi ha il link lo importa e lo rilegge ogni {RILETTURA_MINUTI} minuti. Per ogni link scegli se vede solo gli orari occupati o anche titoli e luoghi.
      </p>
      <Button id="btn-condivisione" variant="outline" size="sm" onClick={() => setAperto(true)}>
        <Share2Icon /> Gestisci i link di condivisione
      </Button>
      <DialogoCondivisione aperto={aperto} onChiudi={() => setAperto(false)} />
    </section>
  );
}
