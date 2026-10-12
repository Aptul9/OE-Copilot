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

// ---- Telegram ------------------------------------------------------------------------------------------------------------
// sendMessage del Bot API: testo semplice (niente parse_mode, così un titolo con simboli non si rompe), al più 4096 caratteri.
// Il token sta solo nel percorso della richiesta: non finisce in un testo d'errore, in un log o in una risposta dell'app.
const TELEGRAM_URL = "https://api.telegram.org";
const TELEGRAM_ATTESA_MS = 10_000;
export const TELEGRAM_MAX_CARATTERI = 4096;

/** Il testo entro il limite di Telegram (conta unità UTF-16, che sono più prudenti dei caratteri), senza spezzare una coppia surrogata. */
export function tronca(testo: string, max = TELEGRAM_MAX_CARATTERI): string {
  if (testo.length <= max) return testo;
  let fine = max - 1;
  const c = testo.charCodeAt(fine - 1);
  if (c >= 0xd800 && c <= 0xdbff) fine--;
  return testo.slice(0, fine) + "…";
}

type RispostaTelegram = { ok?: boolean; parameters?: { retry_after?: unknown; migrate_to_chat_id?: unknown } };

export function canaleTelegram({ attesaMs = TELEGRAM_ATTESA_MS }: { attesaMs?: number } = {}): Canale {
  const token = () => (process.env.TELEGRAM_BOT_TOKEN ?? "").trim();
  const chat = () => (process.env.TELEGRAM_CHAT_ID ?? "").trim();
  const radice = () => (process.env.TELEGRAM_API_URL || TELEGRAM_URL).replace(/\/+$/, "");
  return {
    nome: "telegram",
    configurato: () => Boolean(token() && chat()),
    async invia(testo) {
      let r: Response;
      try {
        r = await fetch(`${radice()}/bot${token()}/sendMessage`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ chat_id: chat(), text: tronca(testo), link_preview_options: { is_disabled: true } }),
          signal: AbortSignal.timeout(attesaMs),
          cache: "no-store",
        });
      } catch (e) {
        // il testo dell'errore di fetch può nominare l'indirizzo, e l'indirizzo contiene il token: si guarda solo il tipo
        const scaduta = e instanceof Error && (e.name === "TimeoutError" || e.name === "AbortError");
        return { ok: false, errore: scaduta ? "Telegram non risponde (tempo scaduto)" : "Telegram non raggiungibile" };
      }
      let corpo: RispostaTelegram | null = null;
      try {
        corpo = JSON.parse(await r.text()) as RispostaTelegram;
      } catch {
        // risposta che non è JSON: conta lo stato HTTP
      }
      if (r.ok) return corpo?.ok === true ? { ok: true } : { ok: false, errore: "Telegram ha dato una risposta che non si capisce" };
      switch (r.status) {
        case 429: {
          const attesa = Number(corpo?.parameters?.retry_after ?? r.headers.get("retry-after"));
          return { ok: false, errore: "Telegram limita gli invii (429)", attesaS: Number.isFinite(attesa) && attesa > 0 ? attesa : 30 };
        }
        case 401:
        case 404:
          // un token scritto male dà 404, uno revocato 401
          return { ok: false, errore: `Telegram non riconosce il bot (${r.status}): controlla TELEGRAM_BOT_TOKEN` };
        case 403:
          return { ok: false, errore: "Telegram: il bot non può scrivere in quella chat (403): va avviato o aggiunto" };
        case 400:
          return {
            ok: false,
            errore: corpo?.parameters?.migrate_to_chat_id
              ? "Telegram: il gruppo è diventato un supergruppo (400): aggiorna TELEGRAM_CHAT_ID"
              : "Telegram ha rifiutato il messaggio (400): controlla TELEGRAM_CHAT_ID",
          };
        default:
          return { ok: false, errore: r.status >= 500 ? `Telegram non disponibile (${r.status})` : `Telegram ha risposto ${r.status}` };
      }
    },
  };
}

/** I canali che il server conosce, configurati o no. */
export const canaliDelServer = (): Canale[] => [canaleTelegram(), canalePush()];
