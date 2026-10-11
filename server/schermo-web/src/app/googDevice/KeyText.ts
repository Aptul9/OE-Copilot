// Che cosa fare di un tasto premuto mentre la tastiera del computer è collegata al telefono.
//
// Un carattere (lettera, cifra, simbolo, anche con Maiusc o AltGr) parte come testo: conta il carattere che la tastiera
// del computer ha prodotto (event.key), non il tasto fisico (event.code). Per codice, con la tastiera italiana il "-"
// arrivava come "/", "_" come "?" e "@" non arrivava affatto, perché i codici sono quelli della tastiera americana.
// Ctrl, Meta e Alt lasciano il tasto al browser (Ctrl+V incolla, F5 ricarica, Ctrl+R ricarica). I tasti che non sono
// caratteri (Invio, Backspace, frecce, Tab...) partono come codici Android, come prima.
export type Azione = 'testo' | 'tasto' | 'niente';

export interface Premuto {
    key: string;
    ctrlKey: boolean;
    altKey: boolean;
    metaKey: boolean;
    altGraph: boolean;
    isComposing: boolean;
}

// tasti che da soli non scrivono niente: il telefono ne vede gli effetti sul carattere, non il tasto
const SOLO_MODIFICATORI = new Set([
    'Shift', 'Control', 'Alt', 'AltGraph', 'Meta', 'OS', 'CapsLock', 'NumLock', 'ScrollLock', 'Fn', 'FnLock',
    'Dead', 'Process', 'Unidentified',
]);

export function classifica(e: Premuto): Azione {
    if (e.isComposing || SOLO_MODIFICATORI.has(e.key)) {
        return 'niente';
    }
    // F1..F12 sono del browser (F5 ricarica, F11 schermo intero, F12 strumenti): sul telefono non servono
    if (/^F\d{1,2}$/.test(e.key)) {
        return 'niente';
    }
    // AltGr arriva da Windows come Ctrl+Alt: produce un carattere, non è una scorciatoia
    const altGr = e.altGraph || (e.ctrlKey && e.altKey);
    if ((e.ctrlKey || e.metaKey || e.altKey) && !altGr) {
        return 'niente';
    }
    return [...e.key].length === 1 ? 'testo' : 'tasto';
}

// Il server sul telefono legge un testo per volta, di lunghezza limitata: i testi lunghi (un incolla) si spezzano
// per caratteri interi, mai in mezzo a una coppia di surrogati.
export function spezza(testo: string, massimo = 200): string[] {
    const pezzi: string[] = [];
    let corrente = '';
    let lunghezza = 0;
    for (const c of testo) {
        if (lunghezza === massimo) {
            pezzi.push(corrente);
            corrente = '';
            lunghezza = 0;
        }
        corrente += c;
        lunghezza++;
    }
    if (corrente) {
        pezzi.push(corrente);
    }
    return pezzi;
}
