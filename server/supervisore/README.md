# Supervisore del telefono virtuale

Il lavoro del servizio systemd `oe-copilot-sincronizza` (`strumenti/sincronizza_telefono.sh` e `strumenti/rileva_accessi.py`) in un servizio Node.js: un giro ogni 35 secondi tiene aggiornati i calendari del telefono, scrive `stato/battito.json` e ogni 2 giri `stato/accessi.json`. Parla col telefono con adb sulla rete di compose (`telefono:5555`) invece di `docker exec`. Facoltativo e non ancora il predefinito: servizio `supervisore` di `docker-compose.yml` (solo con `--profile supervisore`), GUIDA.md sezioni 4.5 e 6.

## Prove

```bash
npm ci --ignore-scripts --no-audit --no-fund
npm run typecheck
npm test
```

- `test/ciclo.test.ts`: il giro contro `strumenti/sincronizza_telefono.sh`. Un telefono inventato (`test/telefono-simulato.ts`) risponde allo script, con `docker`, `sleep` e `date` finti, e al ciclo in TypeScript: i comandi al telefono, le attese e il battito devono essere gli stessi (`test/golden/`). Per rigenerare i golden dallo script vero: `bash test/script-finto/tutti.sh test/golden` (bash con coreutils GNU, Git Bash va bene).
- `test/differenziale.test.ts`: il rilevatore contro `strumenti/rileva_accessi.py` (python3 serve), su dati inventati (`test/differenziale/casi.ts`): `accessi.json` e `google-sync.json` devono uscire uguali byte per byte. L'unica differenza voluta è `google-dump-utf8-invalido`.
- `test/rileva-accessi.test.ts`: i casi di `strumenti/prova_rileva_accessi.py`, con le stesse attese.

## Variabili d'ambiente

Quelle dello script e del rilevatore, con gli stessi valori di partenza: `OGNI` (30), `ACCESSI_OGNI` (2), `GOOGLE_OGNI` (4), `TOCCO_OGNI` (10, 0 spegne), `PAUSA_SCHERMO_SCADE` (60), `OUTLOOK_GIALLO_S` (600), `OUTLOOK_ROSSO_S` (1800), `GOOGLE_GIALLO_S` (900), `GOOGLE_ROSSO_S` (2700). Nuove: `TELEFONO` (indirizzo adb, `telefono:5555`), `OE_DIR` (`/opt/oe-copilot`), `ADB_HOST` e `ADB_PORT` (il server adb locale, `127.0.0.1:5037`), `ADB_TIMEOUT_S` (30). Un valore che non si legge vale quello di partenza e l'avviso finisce nel log.

## Cartelle

- `src/ciclo.ts`: il giro, `src/telefono.ts`: adb (e la guardia delle sole letture per le prove sul telefono in funzione), `src/rilevatore/`: il rilevatore, `src/python.ts`: le regole di Python su cui il rilevatore si appoggia.
- Un container senza root: utente 10080 (quello di Outlook sul telefono), `./telefono` in sola lettura, `./stato` scrivibile da lui.
