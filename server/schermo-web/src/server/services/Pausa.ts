import { unlinkSync, writeFileSync } from 'fs';
import type WS from 'ws';

// Mentre un browser guarda o comanda il telefono, il servizio di sincronizzazione (strumenti/sincronizza_telefono.sh)
// non deve portare Outlook in primo piano: tiene il file PAUSA fresco (lo riscrive ogni 10 s) finché c'è almeno un
// flusso aperto e lo toglie quando se ne va l'ultimo. Se il servizio muore il file invecchia da solo e la
// sincronizzazione lo ignora dopo 60 s. Ogni flusso è sorvegliato con un ping ogni 15 s: un browser sparito senza
// chiudere (portatile chiuso, rete caduta) viene chiuso dopo il secondo giro senza risposta e non tiene ferma
// la sincronizzazione. Senza PAUSA_FILE il servizio non scrive niente.
const FILE = process.env.PAUSA_FILE || '';
const SCRIVI_MS = Number(process.env.PAUSA_SCRIVI_MS) || 10_000;
const PING_MS = Number(process.env.PAUSA_PING_MS) || 15_000;

let aperti = 0;
let timer: ReturnType<typeof setInterval> | undefined;

function scrivi(): void {
    try {
        writeFileSync(FILE, `${Date.now()}\n`);
    } catch (e) {
        console.error(`PAUSA: ${(e as Error).message}`);
    }
}

function togli(): void {
    try {
        unlinkSync(FILE);
    } catch {
        // non c'era
    }
}

export function conteggio(): number {
    return aperti;
}

export function aperto(): void {
    if (!FILE || aperti++ > 0) {
        return;
    }
    scrivi();
    timer = setInterval(scrivi, SCRIVI_MS);
}

export function chiuso(): void {
    if (!FILE || aperti === 0 || --aperti > 0) {
        return;
    }
    clearInterval(timer);
    timer = undefined;
    togli();
}

export function sorveglia(ws: WS): void {
    let vivo = true;
    ws.on('pong', () => {
        vivo = true;
    });
    const ping = setInterval(() => {
        if (!vivo) {
            ws.terminate();
            return;
        }
        vivo = false;
        ws.ping();
    }, PING_MS);
    aperto();
    ws.once('close', () => {
        clearInterval(ping);
        chiuso();
    });
}

if (FILE) {
    process.on('exit', togli);
}
