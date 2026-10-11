# OE Copilot

OE Copilot **unisce in un calendario solo i calendari di tutti gli account** (aziendali e personali), con un colore per azienda. Le caselle le legge un **agente su un server**, da un telefono Android virtuale con le app ufficiali (Outlook e i servizi Google) in cui gli accessi si fanno una volta sola. Si vedono **nel browser**, dietro una pagina di accesso, e con l'**app per Windows**, che aggiunge i **promemoria delle riunioni** come notifica di Windows e, se vuoi, su **Telegram**. C'è anche una sezione **Timesheet** con le schede dei gestionali delle aziende.

Nessuna registrazione su Azure e nessun consenso dell'amministratore: gli accessi stanno nel telefono del server, e sul PC Outlook non serve per leggere i calendari. Installazione passo per passo e perché delle scelte: **[GUIDA.md](GUIDA.md)**.

## Nel browser

Il server serve la pagina dietro HTTPS e una pagina di accesso: sul PC non si installa niente. Come si mette in piedi il server: *Caselle lette dal server* qui sotto e **GUIDA.md**, sezione 4.7.

## App per Windows (facoltativa)

Aggiunge i promemoria, l'avvio con Windows e il pulsante *Apri in Outlook*. Legge le caselle dal server attraverso un tunnel SSH (*Caselle lette dal server*).

### Requisiti

- Windows 10 o 11.
- **Python 3.12** (da python.org).
- Runtime **Microsoft Edge WebView2** (di norma già presente su Windows 11).
- Connessione a internet al primo avvio: il calendario grafico (FullCalendar) arriva da una CDN.
- Un client `ssh` (OpenSSH di Windows) e un server raggiungibile senza password.
- **Outlook desktop classico**, solo per il pulsante *Apri in Outlook* (non il "Nuovo Outlook").

### Installazione

Dalla cartella dell'app, in un prompt dei comandi:

```bat
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python strumenti\applica_patch_pywebview.py
```

L'ultimo comando applica a pywebview due correzioni di cui l'app ha bisogno (icona nella barra
delle applicazioni, bordi della finestra senza cornice). Va ripetuto se si reinstalla pywebview.

Poi, facoltativo ma consigliato: copia `personalizzazione.esempio.json` come
`personalizzazione.json` e mettici le tue aziende (vedi sotto).

### Avvio

- Prima volta: doppio clic su **`start.bat`**. Gli errori finiscono in `debug.log`.
- Uso quotidiano: **`Calendar Hub (silenzioso).vbs`**, senza finestre di prompt.
- Avvio con Windows: si attiva da **Impostazioni** nell'app, oppure con un collegamento al file
  `.vbs` nella cartella `shell:startup`.

Se il server non risponde l'app mostra l'ultima lettura salvata e riprova da sola.

---

## Personalizzazione

Nomi delle aziende, ordine dei calendari, sigle e schede del Timesheet **non stanno nel codice**:
stanno in `personalizzazione.json`, accanto all'app. Quel file è escluso dal repository, perché
contiene i dati di chi usa l'app. Senza il file l'app funziona lo stesso e ogni calendario prende
il nome dal dominio della sua casella. Nel browser lo stesso file sta sul server, in
`ui-dati/personalizzazione.json` (GUIDA.md, 4.7).

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
| `agente_remoto` | Come l'app per Windows raggiunge il server che legge le caselle (vedi sotto) |
| `interfaccia` | Sigle e loghi, calendari secondari (sbiaditi), calendari senza "Apri in Teams", schede Timesheet |

I loghi si indicano come `"logos/nome.png"`: per l'app per Windows stanno nella cartella `web/logos/`, nel browser sul server in `ui-dati/logos/` (GUIDA.md, 4.7).
Dopo una modifica a `personalizzazione.json` riavvia l'app.

### Calendari Google

Un calendario Google si vede in due modi:

1. **Link iCal** (il più semplice): in Google Calendar, *Impostazioni → il calendario → Indirizzo segreto in formato iCal*; nell'app, **Gestisci calendari → Aggiungi un calendario da link**. Il link vale come una password e resta solo in `config.json` su questo PC (nel browser, in `ui-dati/config.json` sul server).
2. **Dal telefono del server**, se l'amministratore ha disattivato il link iCal: l'account Google si aggiunge nel telefono virtuale (*Impostazioni → Password e account*) e i servizi Google ne sincronizzano il calendario, che arriva come quelli di Outlook (GUIDA.md, 4.4).

### Aziende inoltrate

Se un account si apre solo dal browser, un flusso (per esempio Power Automate) può copiarne le riunioni nel calendario di un'altra casella con un prefisso nel titolo, come `Copia: Riunione`. In `aziende_inoltrate` indichi la casella che le riceve (`via`), il prefisso (`prefisso`, espressione regolare) e, facoltativo, il `colore` da dare all'azienda alla prima assegnazione. L'app mostra le copie come l'azienda d'origine e toglie il prefisso dal titolo. Il pallino dell'azienda segue lo stato della casella che le ospita: che il flusso che crea le copie sia vivo, da qui non si può sapere.

### Caselle lette dal server

Le caselle le legge un **agente** su un server Linux, e l'app per Windows e il browser gliele chiedono. Il codice dell'agente e di tutto il server è nella cartella `server/`. Per l'installazione completa passo per passo (server e app per Windows) e il perché delle scelte, vedi **[GUIDA.md](GUIDA.md)**.

Come funziona: sul server gira un telefono Android virtuale ([redroid](https://github.com/remote-android/redroid-doc), in Docker) con le app ufficiali, **Outlook** per le caselle Microsoft e i **servizi Google** per i calendari Google, in cui si fanno gli accessi una volta. Le app copiano gli appuntamenti nel calendario del telefono; l'agente legge quel calendario e lo espone con una piccola API, in ascolto **solo sul localhost del server**. L'app per Windows la raggiunge con un **tunnel SSH** che apre da sola: l'accesso passa dalla tua chiave SSH, più una chiave dell'API. Il browser passa da HTTPS e dalla pagina di accesso (servizi `ui`, `gateway` e `caddy-ui`, GUIDA.md, 4.7); il telefono si vede e si comanda dalla pagina `/telefono/` (GUIDA.md, 4.8), dietro lo stesso accesso.

Sul server (provato su Ubuntu ARM64; servono Docker e il modulo del kernel `binder_linux`):

1. copia `server/` in `/opt/oe-copilot`; crea `.env` da `.env.esempio` con una `API_KEY` lunga e casuale e `agent/fonti.json` da `agent/fonti.esempio.json` con le tue caselle;
2. `docker compose up -d telefono agent` (costruisce l'immagine del telefono, con i servizi Google), poi `strumenti/prepara_telefono.sh` installa Outlook (l'APK sta nella release `apk`) e mette a posto il telefono. Gli accessi si fanno dal telefono: in Outlook per le caselle Microsoft, in *Impostazioni → Password e account* per quelle Google. Lo schermo si vede dalla pagina `/telefono/` o dal PC con [scrcpy](https://github.com/Genymobile/scrcpy) attraverso un tunnel SSH verso la porta 5555. In Outlook va acceso, casella per casella, *Impostazioni → Calendario → Sincronizza calendari*;
3. installa `strumenti/oe-copilot-sincronizza.service` come servizio systemd: a ogni giro, circa ogni 35 secondi, forza la sincronizzazione del telefono (da solo Android la farebbe ogni 15 minuti) e lo tiene sbloccato per Outlook (GUIDA.md, 4.5).

Sul PC, per l'app per Windows:

1. il server dev'essere raggiungibile con `ssh` **senza password** (chiave SSH);
2. salva la chiave dell'API, cifrata per il tuo utente Windows. Non passa mai dallo schermo:
   ```bat
   ssh utente@server "grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2" | .venv\Scripts\python agente_remoto.py imposta-chiave
   ```
3. in `personalizzazione.json` aggiungi `"agente_remoto": {"ssh": "utente@server"}` (o un alias di `~/.ssh/config`); riavvia l'app. `.venv\Scripts\python agente_remoto.py prova` dice se il server risponde.

Senza la voce `ssh` l'app non legge nessuna casella.

Cosa cambia nell'app: il pallino di ogni azienda dice anche se sul server l'aggiornamento è
fermo; se il server non risponde l'app mostra l'ultima lettura salvata e riprova da sola;
*Apri in Outlook* cerca l'evento nell'Outlook del PC per ora e titolo. Limiti: gli appuntamenti
che esistono solo nell'Outlook del PC (calendari "solo questo computer") e i compleanni dei
contatti non ci sono, e le riunioni annullate spariscono invece di restare come "Annullato:".

Un accesso può scadere sul server (password cambiata, sessione revocata): il calendario sul telefono resterebbe fermo e la lettura riuscirebbe lo stesso, col pallino verde su dati vecchi. Per evitarlo `strumenti/rileva_accessi.py` (lanciato dal servizio di sincronizzazione) scrive in `stato/accessi.json` lo stato di ogni casella, che l'agente usa per il pallino, senza mai leggere un token: accesso scaduto in Outlook (rosso), Outlook che non conferma la sincronizzazione (giallo, poi rosso), telefono bloccato (giallo: le cancellazioni e le modifiche arrivano in ritardo), calendari Google che non sincronizzano (giallo, poi rosso). Soglie e dettagli: GUIDA.md, sezione 6.

---

## Promemoria

In **Impostazioni → Promemoria**: minuti di anticipo (anche più di uno, es. `10, 60`) e notifica
di Windows.

**Telegram:** crea un bot con **@BotFather** (`/newbot`), scrivigli un messaggio, ricava il tuo
Chat ID con **@userinfobot**; poi in Impostazioni → Telegram spunta *Attiva*, inserisci token e
Chat ID e prova con *Invia notifica di prova*.

I promemoria partono finché l'app è aperta.

## Timesheet

La sezione Timesheet mostra una scheda per azienda (`interfaccia.schede_timesheet`): un clic
apre il gestionale (`url`) in Chrome.

Una scheda può invece richiamare una funzione scritta apposta (`azione`), tramite le
**estensioni personali**: codice per una persona sola, che resta fuori dal repository.

- `personale/estensione.py` definisce `METODI_API = {"nome": funzione}`: le funzioni diventano
  chiamabili dall'interfaccia (ricevono l'API come primo argomento).
- `web/personale/estensione.js` chiama `registraEstensione({setup, azioni})`: `setup` prepara
  ciò che serve all'avvio, `azioni` associa il nome di un'azione a una funzione.
- `web/personale/estensione.css`, facoltativo, per gli stili.

Se la cartella `personale` non c'è, l'app funziona senza.

---

## Controllo di salute

```bat
.venv\Scripts\python verifica.py
.venv\Scripts\python verifica.py --avvio
```

Verifica codice, interfaccia, lettura delle caselle dal server, coerenza dei pallini di stato e che nessun dato di `personalizzazione.json` sia finito nel codice. Con `--avvio` lancia anche l'app e controlla
che l'interfaccia parta davvero. Esce con codice 1 se qualcosa è rosso.

## File principali

| File | Cosa fa |
|------|---------|
| `app.py` | Finestra desktop e ponte con l'interfaccia |
| `config.py` | Impostazioni, colori, lettura di `personalizzazione.json` |
| `outlook_com.py` | Apertura di un appuntamento in Outlook (COM/MAPI): pulsante *Apri in Outlook* |
| `agente_remoto.py` | Lettura delle caselle dall'agente sul server, attraverso un tunnel SSH |
| `caselle.py` | Regole comuni: di che azienda è una riunione, doppioni, colori |
| `eventi.py` | Unione delle sorgenti, doppioni, stato di ogni azienda |
| `disponibilita.py` | Pagina pubblica di disponibilità: ogni impegno ridotto a un blocco "Occupato" |
| `ics_feed.py` | Calendari da link iCal e festività italiane |
| `local_events.py` | Eventi aggiunti a mano nell'app |
| `reminders.py` | Promemoria: notifiche Windows e Telegram |
| `teams_link.py` | "Apri in Teams" sull'account giusto |
| `verifica.py` | Controllo di salute |
| `web/` | Interfaccia dell'app per Windows (FullCalendar) |
| `strumenti/` | Correzione di pywebview e relativa licenza |
| `server/` | Il server: agente, telefono virtuale, interfaccia nel browser (`web-ts/`, `ui/`, `caddy-ui/`), schermo del telefono (`schermo-web/`) e script (`strumenti/`); vedi *Caselle lette dal server* |

Restano sul PC e fuori dal repository: `personalizzazione.json`, `config.json`, le cache
(`calendars_cache.json`, `events_cache.json`, `ics_cache/`, `agente_caselle.json`), la chiave
cifrata dell'agente (`agente_chiave.bin`), gli eventi personali, i log e la cartella
`personale/`.

## Se qualcosa non va

Apri **`debug.log`**: contiene gli avvii, i tempi di lettura e gli eventuali errori.

- **La finestra resta vuota**: di solito manca la connessione al primo avvio.
- **Non compare un calendario appena aggiunto**: **Gestisci calendari → Ricerca calendari nuovi** rilegge l'elenco dal server (è memorizzato). Se manca ancora: sul telefono Outlook passa solo i calendari spuntati, con *Sincronizza calendari* acceso per quella casella, e l'agente legge solo quelli elencati in `agent/fonti.json` (GUIDA.md, 4.2 e 4.4).
- **"Impossibile parlare con Outlook"** (pulsante *Apri in Outlook*): apri Outlook classico e riprova.
- **"Il server dei calendari non risponde"** (caselle lette da un server): prova
  `.venv\Scripts\python agente_remoto.py prova`; il motivo di un tunnel che non parte è in
  `agente_tunnel.log`. Un pallino giallo o rosso su un'azienda dice nel suo suggerimento cosa
  è fermo sul server.
- **Outlook chiede "Scegli profilo"**: ci sono più profili e nessuno è predefinito. Indica quello
  da usare in `config.json` (`"outlook_profile": "NomeProfilo"`) o impostane uno predefinito in
  *Pannello di controllo → Posta → Mostra profili*.
