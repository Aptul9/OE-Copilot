import { canalePush } from "./promemoria-push";

// I canali dei promemoria: dove il server manda il testo. Un canale dice se è configurato (le chiavi stanno nell'ambiente del server,
// mai nella configurazione dell'app) e sa inviare un testo. L'esito dice solo se è andata e, se no, perché, con una frase fissa:
// niente di quello che il servizio ha risposto, e mai la chiave.

export type Esito = { ok: true } | { ok: false; errore: string; attesaS?: number };

export interface Canale {
  nome: string;
  /** Pronto a mandare: la chiave o le chiavi del server ci sono (e, se serve, chi riceve). */
  configurato(): boolean;
  invia(messaggio: string): Promise<Esito>;
  /** Quello che la pagina può sapere del canale oltre a configurato: mai una chiave segreta, mai un indirizzo di chi riceve. */
  dettagli?(): Record<string, unknown>;
}

// ---- Il canale di prova ---------------------------------------------------------------------------------------------------
export type CanaleFinto = Canale & {
  /** I testi inviati. */
  inviati: string[];
  /** Quante volte si è provato a inviare, compresi i tentativi falliti. */
  tentativi: number;
  /** Gli esiti dei prossimi invii, in ordine; finiti questi, ogni invio riesce. */
  esiti: Esito[];
  configuratoOra: boolean;
};

export function canaleFinto(nome = "finto", configurato = true): CanaleFinto {
  const c: CanaleFinto = {
    nome,
    inviati: [],
    tentativi: 0,
    esiti: [],
    configuratoOra: configurato,
    configurato: () => c.configuratoOra,
    async invia(testo) {
      c.tentativi++;
      const esito = c.esiti.shift() ?? { ok: true };
      if (esito.ok) c.inviati.push(testo);
      return esito;
    },
  };
  return c;
}

/** I canali che il server conosce, configurati o no. */
export const canaliDelServer = (): Canale[] => [canalePush()];
