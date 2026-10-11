import Size from '../Size';

// Qualità di partenza del flusso, uguale per tutti i lettori. Misurate sul telefono vero (decisione
// 2026-10-11-oe-copilot-phone-browser): a 15 fotogrammi al secondo e 1 Mbit/s con la risoluzione intera (720x1280, testo
// leggibile e tocchi precisi) il telefono arriva a 1,3 core nel caso peggiore (tendina trascinata di continuo),
// a schermo fermo quasi 0. Chi vuole altro lo cambia da "Configure stream": la scelta resta nel browser.
export const DEFAULT_STREAM = {
    lockedVideoOrientation: -1,
    bitrate: 1048576,
    maxFps: 15,
    iFrameInterval: 5,
    bounds: new Size(720, 1280),
    sendFrameMeta: false,
};
