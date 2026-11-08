import { condividi } from "@/lib/condividi";
import { registraAccesso, trovaPerToken } from "@/lib/core/condivisione";
import { leggiPerCondivisione } from "@/lib/core/condivisione-eventi";

// Pubblica, senza sessione: /condividi/<testo segreto>.ics è il calendario di un link di condivisione (lib/condividi.ts). Il testo segreto è l'unica
// chiave: un indirizzo che non vale riceve lo stesso 404 di un percorso che non c'è. Chi è dentro il sito gestisce i link da Impostazioni > Condivisione.
export const dynamic = "force-dynamic";

const run = (req: Request) => condividi(req, { verifica: trovaPerToken, leggi: leggiPerCondivisione, accesso: registraAccesso });

export { run as GET, run as HEAD };
