# Supervisore del telefono virtuale

Il servizio Node.js che tiene aggiornato il telefono. A ogni giro, circa ogni 35 secondi, forza la sincronizzazione dei calendari e scrive `stato/battito.json`; ogni 2 giri lancia il rilevatore degli accessi scaduti, che scrive `stato/accessi.json`. Parla col telefono con adb sulla rete di compose (`telefono:5555`). È il servizio `supervisore` di `docker-compose.yml`, sempre acceso e ricreato a ogni deploy (GUIDA.md, sezioni 4.5 e 6).

## Prove

```bash
npm ci --ignore-scripts --no-audit --no-fund
npm run typecheck
npm test
```
- `test/ciclo.test.ts`: il giro, con un telefono inventato (`test/telefono-simulato.ts`), contro le sequenze registrate in `test/golden/ciclo-<scenario>.txt`.
- `test/differenziale.test.ts`: il rilevatore contro `test/golden/rilevatore-accessi.json`, sui casi di `test/differenziale/casi.ts`. `accessi.json` e `google-sync.json` devono uscire uguali byte per byte.
- `test/rilevatore.test.ts`, `test/rileva-accessi.test.ts`, `test/telefono.test.ts`: i pezzi.

## Variabili d'ambiente

`OGNI` (30), `ACCESSI_OGNI` (2), `GOOGLE_OGNI` (4), `TOCCO_OGNI` (10, 0 spegne), `PAUSA_SCHERMO_SCADE` (60), `OUTLOOK_GIALLO_S` (600), `OUTLOOK_ROSSO_S` (1800), `GOOGLE_GIALLO_S` (900), `GOOGLE_ROSSO_S` (2700), `TELEFONO` (`telefono:5555`), `OE_DIR` (`/opt/oe-copilot`), `ADB_HOST` e `ADB_PORT` (`127.0.0.1:5037`), `ADB_TIMEOUT_S` (30). Un valore che non si legge vale quello di partenza, con un avviso nel log.

## Trappole

- I file di `test/golden/` sono dati fissi e non si rigenerano: una differenza è un comportamento cambiato.
- I casi nuovi del rilevatore vanno in `test/rilevatore.test.ts`, non in `casi.ts`: il confronto controlla che i nomi dei casi siano quelli registrati.
- Il container gira come l'utente 10080, quello di Outlook sul telefono: solo lui e root leggono database e log di Outlook. `./telefono` è montato in sola lettura, `./stato` deve essere suo.
