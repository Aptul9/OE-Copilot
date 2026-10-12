# OE Copilot

OE Copilot **unisce in un calendario solo i calendari di tutti gli account** (aziendali e personali), con un colore per azienda. Le caselle le legge un **agente su un server**, da un telefono Android virtuale con le app ufficiali (Outlook e i servizi Google) in cui gli accessi si fanno una volta sola. Si vedono **nel browser**, dietro una pagina di accesso: sul PC non si installa niente. C'è anche una sezione **Timesheet** con le schede dei gestionali delle aziende.

Nessuna registrazione su Azure e nessun consenso dell'amministratore: gli accessi stanno nel telefono del server. Installazione passo per passo e perché delle scelte: **[GUIDA.md](GUIDA.md)**. L'app per Windows (promemoria, avvio con Windows, *Apri in Outlook*) è stata ritirata l'11/10/2026: GUIDA.md, sezione 3.

## Nel browser

Il server serve la pagina dietro HTTPS e una pagina di accesso. Come si mette in piedi il server: *Caselle lette dal server* qui sotto e **GUIDA.md**, sezione 4.7. L'accesso può avere un secondo fattore, spento di default (il codice di un'app di autenticazione con i codici di recupero, oppure le passkey): variabile `AUTH_2FA` in `.env`, attivazione e recupero in **GUIDA.md**, sezione 4.7.

I promemoria delle riunioni li può mandare il server su Telegram o nelle notifiche del browser, anche a pagina chiusa: facoltativo e spento di norma, **GUIDA.md**, sezione 4.9.

---

## Personalizzazione

Nomi delle aziende, ordine dei calendari, sigle e schede del Timesheet **non stanno nel codice**:
stanno in `personalizzazione.json`, sul server in `ui-dati/` (GUIDA.md, 4.7). Quel file è escluso dal
repository, perché contiene i dati di chi usa l'app. Senza il file l'app funziona lo stesso e ogni
calendario prende il nome dal dominio della sua casella.

`personalizzazione.esempio.json` contiene un esempio di ogni voce, con le istruzioni. In breve:

| Voce | A cosa serve |
|------|--------------|
| `nomi_per_indirizzo` | Nome da mostrare per una casella precisa |
| `nomi_per_dominio` | Nome da mostrare per tutte le caselle di un dominio |
| `domini_aziendali` | Domini di aziende vere: un invito organizzato da questi domini prende il nome dell'azienda anche se Outlook lo archivia nel calendario di un'altra casella |
| `caselle_di_colleghi` | Caselle di altre persone aperte in Outlook: i loro eventi restano loro |
| `ordine_calendari` | Ordine dei calendari nella barra laterale |
| `azienda_per_cartella` | Cartelle calendario dedicate a un'azienda, riconosciute dal nome |
| `account_nascosti` | Caselle da non mostrare mai |
| `mostra_tutto_il_giorno` | `true` mostra gli eventi di un giorno intero delle caselle (per esempio i "Home" di Google Calendar); senza la voce non si mostrano. Le festività non cambiano |
| `aziende_inoltrate` | Aziende le cui riunioni arrivano copiate in un'altra casella (vedi sotto) |
| `interfaccia` | Sigle e loghi, calendari secondari (sbiaditi), calendari senza "Apri in Teams", schede Timesheet |

I loghi si indicano come `"logos/nome.png"` e stanno sul server in `ui-dati/logos/` (GUIDA.md, 4.7); l'icona
degli *Eventi personali* (`personale.png`) la porta l'immagine del gateway. Il gateway legge i loghi da disco e rilegge
`personalizzazione.json` quando cambia: dopo una modifica non si riavvia niente.

### Calendari Google

Un calendario Google si vede in due modi:

1. **Link iCal** (il più semplice): in Google Calendar, *Impostazioni → il calendario → Indirizzo segreto in formato iCal*; nell'app, **Gestisci calendari → Aggiungi un calendario da link**. Il link vale come una password e resta solo in `ui-dati/config.json` sul server.
2. **Dal telefono del server**, se l'amministratore ha disattivato il link iCal: l'account Google si aggiunge nel telefono virtuale (*Impostazioni → Password e account*) e i servizi Google ne sincronizzano il calendario, che arriva come quelli di Outlook (GUIDA.md, 4.4).

### Aziende inoltrate

Se un account si apre solo dal browser, un flusso (per esempio Power Automate) può copiarne le riunioni nel calendario di un'altra casella con un prefisso nel titolo, come `Copia: Riunione`. In `aziende_inoltrate` indichi la casella che le riceve (`via`), il prefisso (`prefisso`, espressione regolare) e, facoltativo, il `colore` da dare all'azienda alla prima assegnazione. L'app mostra le copie come l'azienda d'origine e toglie il prefisso dal titolo. Il pallino dell'azienda segue lo stato della casella che le ospita: che il flusso che crea le copie sia vivo, da qui non si può sapere.

### Caselle lette dal server

Le caselle le legge un **agente** su un server Linux, e il browser gliele chiede attraverso il gateway. Il codice dell'agente e di tutto il server è nella cartella `server/`. Per l'installazione completa passo per passo e il perché delle scelte, vedi **[GUIDA.md](GUIDA.md)**.

Come funziona: sul server gira un telefono Android virtuale ([redroid](https://github.com/remote-android/redroid-doc), in Docker) con le app ufficiali, **Outlook** per le caselle Microsoft e i **servizi Google** per i calendari Google, in cui si fanno gli accessi una volta. Le app copiano gli appuntamenti nel calendario del telefono; l'agente legge quel calendario e lo espone con una piccola API, in ascolto **solo sul localhost del server** e sulla rete di Docker. Il browser passa da HTTPS e dalla pagina di accesso (servizi `gateway` e `caddy-ui`, GUIDA.md, 4.7); il telefono si vede e si comanda dalla pagina `/telefono/` (GUIDA.md, 4.8), dietro lo stesso accesso. Dal PC l'agente si interroga con un tunnel SSH (GUIDA.md, sezione 5).

Sul server (provato su Ubuntu ARM64; servono Docker e il modulo del kernel `binder_linux`):

1. copia `server/` in `/opt/oe-copilot`; crea `.env` da `.env.esempio` con una `API_KEY` lunga e casuale e `agent/fonti.json` da `agent/fonti.esempio.json` con le tue caselle;
2. `docker compose up -d telefono agent` (costruisce l'immagine del telefono, con i servizi Google), poi `strumenti/prepara_telefono.sh` installa Outlook (l'APK sta nella release `apk`) e mette a posto il telefono. Gli accessi si fanno dal telefono: in Outlook per le caselle Microsoft, in *Impostazioni → Password e account* per quelle Google. Lo schermo si vede dalla pagina `/telefono/` o dal PC con [scrcpy](https://github.com/Genymobile/scrcpy) attraverso un tunnel SSH verso la porta 5555. In Outlook va acceso, casella per casella, *Impostazioni → Calendario → Sincronizza calendari*;
3. avvia il supervisore (il servizio `supervisore` di compose, `supervisore/`; prima era il servizio systemd `strumenti/oe-copilot-sincronizza.service`): a ogni giro, circa ogni 35 secondi, forza la sincronizzazione del telefono (da solo Android la farebbe ogni 15 minuti) e lo tiene sbloccato per Outlook (GUIDA.md, 4.5).

Un accesso può scadere sul server (password cambiata, sessione revocata): il calendario sul telefono resterebbe fermo e la lettura riuscirebbe lo stesso, col pallino verde su dati vecchi. Per evitarlo `strumenti/rileva_accessi.py` (lanciato dal servizio di sincronizzazione) scrive in `stato/accessi.json` lo stato di ogni casella, che l'agente usa per il pallino, senza mai leggere un token: accesso scaduto in Outlook (rosso), Outlook che non conferma la sincronizzazione (giallo, poi rosso), telefono bloccato (giallo: le cancellazioni e le modifiche arrivano in ritardo), calendari Google che non sincronizzano (giallo, poi rosso). Soglie e dettagli: GUIDA.md, sezione 6.

---

## Timesheet

La sezione Timesheet mostra una scheda per azienda (`interfaccia.schede_timesheet`): un clic
apre il gestionale (`url`) in una scheda nuova del browser.

---

## Controlli prima di un push

- `python server/strumenti/controlla_dati_personali.py <personalizzazione.json>`: cerca i dati della personalizzazione nei file del repository e fallisce se ne trova.
- Le prove della Action (`.github/workflows/server.yml`): sintassi, prove Python in `server/strumenti/` (il rilevatore degli accessi e l'agente su un calendario di Android inventato, `prova_agente_golden.py`), gateway (tipi, `npm test` con le risposte registrate in `server/strumenti/golden/`, build; GUIDA.md, sezione 7), schermo, immagini e il cancello dell'accesso.

## File principali

| File | Cosa fa |
|------|---------|
| `genera_icona.py`, `icon.png` | L'icona (tre calendari sovrapposti) e lo script che la disegna |
| `server/web-ts/logos-predefiniti/` | `personale.png`, l'icona degli *Eventi personali*: il gateway la serve se in `ui-dati/logos/` non c'è un file con lo stesso nome |
| `server/` | Il server: agente, telefono virtuale, interfaccia nel browser (`web-ts/`, `caddy-ui/`), schermo del telefono (`schermo-web/`) e script (`strumenti/`); vedi *Caselle lette dal server*. La logica dell'app è in TypeScript, nel gateway (`web-ts/src/lib/core/`): Python resta solo nell'agente (`agent/`) e negli script |

Restano sul server (`ui-dati/`) e fuori dal repository: `personalizzazione.json`, le cache
(`events_cache.json`, `ics_cache/`, `agente_caselle.json`) e il database `gateway.db`, che tiene gli accessi
e lo stato dell'app (configurazione ed eventi personali; `config.json` e `local_events.json` ne sono la copia,
riscritta a ogni modifica).

## Se qualcosa non va

I log dei servizi: `docker logs oe-copilot-<servizio>` sul server (GUIDA.md, sezione 7).

- **La pagina resta vuota o chiede di nuovo l'accesso**: il cookie di sessione dura 30 giorni; dopo un cambio di `ADMIN_PASSWORD` e un riavvio del gateway le sessioni si chiudono.
- **Non riesco più a entrare con il secondo fattore** (telefono perso, codici persi): sul server, in `.env`, `AUTH_2FA=off`, poi `sudo docker compose up -d gateway` dalla cartella `/opt/oe-copilot`: basta la password. I passaggi, e come ripartire da zero col codice: GUIDA.md, sezione 4.7.
- **Non compare un calendario appena aggiunto**: **Gestisci calendari → Ricerca calendari nuovi** rilegge l'elenco dal server (è memorizzato). Se manca ancora: sul telefono Outlook passa solo i calendari spuntati, con *Sincronizza calendari* acceso per quella casella, e l'agente legge solo quelli elencati in `agent/fonti.json` (GUIDA.md, 4.2 e 4.4).
- **"Il server dei calendari non risponde"**: l'agente non risponde al gateway; `docker logs oe-copilot-agent` e `docker logs oe-copilot-gateway` dal server, e dal PC la prova della GUIDA.md, sezione 5. Un pallino giallo o rosso su un'azienda dice nel suo suggerimento cosa è fermo sul server.
