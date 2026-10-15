# Agente

Legge in sola lettura il `calendar.db` del telefono e lo serve al gateway con un'API a chiave. È il servizio `agent` di `docker-compose.yml` (container `oe-copilot-agent`, porta `127.0.0.1:3180`), GUIDA.md sezioni 4.4 e 5.

- API: `GET /api/eventi?inizio=...&fine=...`, `GET /api/calendari`, `GET /api/salute`, `POST /api/ics`, con `Authorization: Bearer <API_KEY>`. Cache di 15 secondi, corpo al più 8.000.000 byte.
- Avvio: `node src/main.ts` (Node 26 esegue i `.ts` così come sono, senza dipendenze a runtime). Variabili: `API_KEY` (senza, non parte), `PORTA` (8180), `FONTI` (`/app/fonti.json`), `ANDROID_DB` (`/android-calendario`), `BATTITO` (`/stato/battito.json`), `ACCESSI` (`/stato/accessi.json`), `TZ` (`Europe/Rome`).

## Prove

```bash
npm ci --ignore-scripts --no-audit --no-fund
npm run typecheck
npm test
```
`test/golden-agente.test.ts` confronta il server HTTP vero con le risposte registrate in `../strumenti/golden/`; gli altri file provano i pezzi e il motore delle ricorrenze. La CI rifà il confronto sull'immagine con `test/costruisci-db.ts` e `test/confronta-http.ts`.

## Trappole

- Le risposte registrate sono dati fissi e non si rigenerano: una differenza è un comportamento cambiato, da capire prima di toccare il file.
- `calendar.db` si legge da una copia (con `-journal`, `-wal` e `-shm`) aperta con `PRAGMA query_only = ON`: Android tiene aperto l'originale.
- `src/entita.ts` e `src/windows.ts` sono tabelle generate: non si correggono a mano.
- Byte non ASCII grezzi nel percorso danno 400; il gateway codifica sempre il percorso.
- In compose il container non ha capacità e ha il file system in sola lettura; i file del calendario li legge col gruppo di Android (`group_add`, `CALENDARIO_GID`: GUIDA.md, sezione 5).
