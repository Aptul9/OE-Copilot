"use client";

import { useEffect, useState } from "react";
import { infoApp } from "@/lib/oe/logic";

/** Dentro l'app per Windows e Android: il server a cui è collegata, o null; nel browser non c'è. Letto dopo il primo disegno, così il server e il browser disegnano la stessa cosa. */
export function useInfoApp() {
  const [app, setApp] = useState<ReturnType<typeof infoApp>>(null);
  useEffect(() => setApp(infoApp(window as never)), []);
  return app;
}

/** Il server dell'app e il link per cambiarlo (la pagina di avvio dell'app, con il campo aperto). Fuori dall'app non disegna niente. */
export function ServerApp({ className }: { className?: string }) {
  const app = useInfoApp();
  if (!app) return null;
  return (
    <p id="server-app" className={className ?? "text-sm text-muted-foreground"}>
      Server <b className="font-semibold text-foreground">{app.host}</b> ·{" "}
      <a id="server-app-cambia" href={app.cambiaServer} className="font-medium text-primary underline-offset-4 hover:underline">
        Cambia server
      </a>
    </p>
  );
}
