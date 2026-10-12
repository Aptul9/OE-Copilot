# Supervisore del telefono virtuale

Il servizio Node.js che tiene aggiornato il telefono: un giro ogni 35 secondi forza la sincronizzazione dei calendari, scrive `stato/battito.json` e ogni 2 giri `stato/accessi.json` (il rilevatore degli accessi scaduti). Parla col telefono con adb sulla rete di compose (`telefono:5555`) invece di `docker exec`. È il servizio `supervisore` di `docker-compose.yml`: sempre acceso, parte con `docker compose up` senza profilo e l'aggiornamento automatico lo ricrea a ogni deploy; GUIDA.md sezioni 4.5 e 6. Il lavoro lo faceva un servizio systemd con uno script bash e un rilevatore in Python: non ci sono più e stanno nella cronologia di git.

## Prove

```bash
npm ci --ignore-scripts --no-audit --no-fund
npm run typecheck
npm test
```

- `test/ciclo.test.ts`: il giro contro le sequenze registrate in `test/golden/ciclo-<scenario>.txt`. Un telefono inventato (`test/telefono-simulato.ts`) risponde al ciclo in TypeScript: i comandi al telefono, le attese e il battito devono essere quelli che il vecchio script bash produsse una sola volta, con `docker`, `sleep` e `date` finti. I file non si rigenerano: sono dati di prova fissi, e lo script è nella cronologia di git.
- `test/differenziale.test.ts`: il rilevatore contro le uscite registrate del rilevatore in Python (`test/golden/rilevatore-accessi.json`, registrato una sola volta sui dati inventati di `test/differenziale/casi.ts`): `accessi.json` e `google-sync.json` devono uscire uguali byte per byte, con la stessa riga di riepilogo e gli stessi comandi al telefono. Non serve Python. L'unica differenza voluta è `google-dump-utf8-invalido`, controllata riga per riga. I casi nuovi del rilevatore vanno in `test/rilevatore.test.ts`, non in `casi.ts`: il confronto controlla che i nomi dei casi siano quelli registrati.
- `test/rileva-accessi.test.ts`: i casi della vecchia prova Python del rilevatore, con le stesse attese.

## Variabili d'ambiente

`OGNI` (30), `ACCESSI_OGNI` (2), `GOOGLE_OGNI` (4), `TOCCO_OGNI` (10, 0 spegne), `PAUSA_SCHERMO_SCADE` (60), `OUTLOOK_GIALLO_S` (600), `OUTLOOK_ROSSO_S` (1800), `GOOGLE_GIALLO_S` (900), `GOOGLE_ROSSO_S` (2700): le stesse del vecchio script e del rilevatore in Python, con gli stessi valori di partenza. Aggiunte col supervisore: `TELEFONO` (indirizzo adb, `telefono:5555`), `OE_DIR` (`/opt/oe-copilot`), `ADB_HOST` e `ADB_PORT` (il server adb locale, `127.0.0.1:5037`), `ADB_TIMEOUT_S` (30). Un valore che non si legge vale quello di partenza e l'avviso finisce nel log.

## Cartelle

- `src/ciclo.ts`: il giro, `src/telefono.ts`: adb (e la guardia delle sole letture per le prove sul telefono in funzione), `src/rilevatore/`: il rilevatore, `src/python.ts`: le regole di Python su cui il rilevatore si appoggia.
- Un container senza root: utente 10080 (quello di Outlook sul telefono), `./telefono` in sola lettura, `./stato` scrivibile da lui.
