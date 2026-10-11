# Agente in TypeScript: il nucleo delle ricorrenze

Prova di fattibilità, non un servizio: il solo calcolo delle ricorrenze di `server/agent` (Python) riscritto in TypeScript, per misurare se dà le stesse risposte. Non c'è il server HTTP né i passaggi che tolgono i doppioni.

- `espandiRigheAndroid(righe, inizio, fine)`: le righe di `Events` di `calendar.db` rimesse in serie (`_quando`, la durata `P900S`, le colonne RRULE/RDATE/EXDATE anche come `TZID=Europe/Rome;2026...`, le occorrenze annullate che diventano EXDATE, le modifiche con RECURRENCE-ID) e espanse. Fa quello che fa `server/agent/fonte_android.py` fra la query SQL e i doppioni.
- `espandiIcs(testo, inizio, fine)` e `validaIcs(testo)`: la stessa uscita di `server/agent/ics.py` (`POST /api/ics`).
- `inizio` e `fine` sono ore locali di Roma senza fuso (`2026-10-19T00:00:00`), come le riceve l'agente.

Sorgenti: `src/tempo.ts` (date e orari come li tratta Python), `src/regola.ts` (RRULE come dateutil.rrule), `src/serie.ts` (serie, esclusioni, modifiche come recurring_ical_events 3.8.2), `src/ics.ts` (righe ICS come icalendar 7.3.0 e correzione di X-WR-TIMEZONE), `src/android.ts` (le righe del telefono). Usa il `Temporal` di Node 26, nessuna dipendenza a runtime.

Prove (`npm test`, Node 26): il golden dell'agente (`../strumenti/golden/agente.json`, `ics/*.ics`, righe in `agente-righe.json`), 342 casi inventati (`ricorrenze-extra.json`) e 288 casuali (`ricorrenze-casuali.json`), tutti registrati con le librerie Python vere in Linux. Le risposte attese non si ricalcolano qui: si registrano con le stesse librerie nell'immagine dell'agente, su un container usa e getta.

Cosa non fa: i VTIMEZONE definiti nel file ICS (valgono i nomi IANA e la tabella dei nomi di Windows di icalendar, `src/windows.ts`), le frequenze orarie e BYWEEKNO.
