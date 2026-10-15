"use client";

import { Share2Icon } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { DialogoCondivisione } from "./DialogoCondivisione";

/** "Condivisione": nelle impostazioni c'è solo il pulsante; i link si gestiscono nella finestra che si apre da qui. */
export function ImpostazioniCondivisione() {
  const [aperto, setAperto] = useState(false);
  return (
    <section id="cfg-condivisione">
      <h3 className="mb-2 font-semibold">Condivisione</h3>
      <Button id="btn-condivisione" variant="outline" size="sm" onClick={() => setAperto(true)}>
        <Share2Icon /> Gestisci i link
      </Button>
      <DialogoCondivisione aperto={aperto} onChiudi={() => setAperto(false)} />
    </section>
  );
}
