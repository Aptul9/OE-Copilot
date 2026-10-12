# Agente in TypeScript

Lo stesso agente di `server/agent` (Python) scritto in TypeScript: stessa API, stesse risposte, lo stesso `calendar.db` del telefono letto in sola lettura. Non è il servizio in uso: sta dietro il profilo `agente-ts` di compose e non parte da solo (GUIDA.md, sezioni 4.4 e 5).

- API: `GET /api/eventi?inizio=...&fine=...`, `GET /api/calendari`, `GET /api/salute`, `POST /api/ics` (chiave `Authorization: Bearer <API_KEY>`, risposte in JSON, cache di 15 secondi sulle finestre, corpo massimo 8 MB), stessi codici di stato e stessi testi di errore di Python.
- Avvio: `node src/main.ts` (Node 26, esegue i sorgenti `.ts` così come sono, nessuna compilazione e nessuna dipendenza a runtime). Variabili come in Python: `API_KEY` (senza, non parte), `PORTA` (8180), `FONTI` (`/app/fonti.json`), `ANDROID_DB` (`/android-calendario`), `BATTITO` (`/stato/battito.json`), `ACCESSI` (`/stato/accessi.json`), `TZ` (`Europe/Rome`).
- Il calendario si legge su una copia di `calendar.db` (con `-journal`, `-wal` e `-shm`) aperta con `node:sqlite` e `PRAGMA query_only=ON`: Android lo tiene aperto e il file originale non si tocca.
- Immagine: `Dockerfile` (`node:26-slim`, utente 10001). In compose il servizio gira a file system in sola lettura, senza capacità, con il gruppo dei file di Android (`group_add`).

## Sorgenti

- `src/server.ts` le quattro rotte, la chiave, la cache, i limiti, i log nello stile di Python; `src/main.ts` l'avvio.
- `src/fonte.ts` la lettura di `calendar.db`: scelta del calendario, dettagli, doppioni, stato di ogni casella (`accessi.json`, `battito.json`).
- `src/iso.ts` (`fromisoformat` di Python 3.12), `src/query.ts` (`urlparse` e `parse_qs`), `src/html.ts` (le entità HTML), `src/pyfmt.ts` (`repr`, `strip`, `json.dumps`, ordine dei testi di Python).
- `src/regola.ts` le RRULE come `dateutil.rrule` (anche HOURLY, MINUTELY, SECONDLY e BYWEEKNO), `src/serie.ts` le serie, le esclusioni e le modifiche come `recurring_ical_events` 3.8.2, `src/tempo.ts` date e orari come li tratta Python, `src/android.ts` le righe del telefono.
- `src/ics.ts` il testo ICS come `icalendar` 7.3.0 e la correzione di X-WR-TIMEZONE di `x-wr-timezone` 2.0.1; `src/vtimezone.ts` e `src/fusi.ts` i VTIMEZONE definiti nel calendario (come `dateutil.tz.tzical`, con la cache dei fusi che in Python vive quanto il processo).
- `src/entita.ts` e `src/windows.ts` sono tabelle generate dalle librerie di Python (`html.entities.html5` di Python 3.12, `windows_to_olson` di icalendar): non si correggono a mano.

## Prove

`npm test` (Node 26): tutto contro le risposte che Python dà nell'immagine dell'agente, registrate in `../strumenti/golden/`. Le risposte attese non si ricalcolano qui: si registrano con le librerie vere su un container usa e getta.

- `golden-agente.test.ts` il golden dell'agente (`agente.json`, `agente-righe.json`, `ics/*.ics`) attraverso il vero server HTTP.
- `agente-unita.test.ts` più di 54000 casi dei pezzi (date, entità, link, percorsi, JSON, ordine dei testi) contro Python: nel repository un campione; `agente-bordi.test.ts` le richieste che nessun cliente normale manda e i modi di non partire.
- `ricorrenze-extra.test.ts`, `ricorrenze-casuali.test.ts`, `ricorrenze-orarie.test.ts` e `ricorrenze-vtimezone.test.ts` il motore delle ricorrenze: casi inventati e casuali, regole orarie e BYWEEKNO, VTIMEZONE (ognuno parte da una cache dei fusi vuota, tranne quelli che provano la cache).
- `confronta-http.ts` e `costruisci-db.ts` servono alla CI: il `calendar.db` inventato del golden e le stesse richieste su un agente in ascolto (l'immagine), da confrontare con le risposte registrate.

## Differenze volute

- Byte non ASCII grezzi (senza `%`) nel percorso della richiesta: 400, perché Node li rifiuta; Python li accetta. Il gateway codifica sempre il percorso.
- Uno spazio dopo la chiave giusta in `Authorization`: passa, perché Node toglie gli spazi in coda al valore di un'intestazione; Python risponde 401.
- Un testo ICS senza a capo non è mai preso per il percorso di un file (Python lo apre se esiste).
- Il resto dell'involucro HTTP: Python risponde `HTTP/1.0` con l'intestazione `Server`, questo `HTTP/1.1` con `Connection: close`.
