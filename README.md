# OE Copilot

App desktop per Windows che **unisce in un calendario solo i calendari di tutti gli account
configurati in Outlook** (aziendali e personali), con un colore per azienda, e manda i
**promemoria delle riunioni** come notifica di Windows e, se vuoi, su **Telegram**.
Ha anche una sezione **Timesheet** con le schede dei gestionali delle aziende.

L'app legge quello che Outlook desktop ha già sul PC: nessun login, nessuna password, nessuna
registrazione su Azure. I dati restano sul PC.

In alternativa le caselle si possono far leggere a un **server** (vedi *Caselle lette da un
server*): gli accessi stanno lì e sul PC Outlook non serve più per il calendario.

---

## Requisiti

- Windows 10 o 11, con **Outlook desktop classico** installato e configurato (non il "Nuovo
  Outlook").
- **Python 3.12** (da python.org).
- Runtime **Microsoft Edge WebView2** (di norma già presente su Windows 11).
- Connessione a internet al primo avvio: il calendario grafico (FullCalendar) arriva da una CDN.

## Installazione

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

## Avvio

- Prima volta: doppio clic su **`start.bat`**. Gli errori finiscono in `debug.log`.
- Uso quotidiano: **`Calendar Hub (silenzioso).vbs`**, senza finestre di prompt.
- Avvio con Windows: si attiva da **Impostazioni** nell'app, oppure con un collegamento al file
  `.vbs` nella cartella `shell:startup`.

Se Outlook è chiuso l'app mostra l'ultima lettura salvata, avvia Outlook in sottofondo e si
aggiorna da sola appena è pronto.

---

## Personalizzazione

Nomi delle aziende, ordine dei calendari, sigle e schede del Timesheet **non stanno nel codice**:
stanno in `personalizzazione.json`, accanto all'app. Quel file è escluso dal repository, perché
contiene i dati di chi usa l'app. Senza il file l'app funziona lo stesso e ogni calendario prende
il nome dal dominio della sua casella.

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
| `calendari_google` | Calendari Google letti direttamente (vedi sotto) |
| `aziende_inoltrate` | Aziende le cui riunioni arrivano copiate in un'altra casella (vedi sotto) |
| `agente_remoto` | Caselle lette da un server invece che da Outlook su questo PC (vedi sotto) |
| `interfaccia` | Sigle e loghi, calendari secondari (sbiaditi), calendari senza "Apri in Teams", schede Timesheet |

I loghi vanno nella cartella `web/logos/` e si indicano come `"logos/nome.png"`.
Dopo una modifica a `personalizzazione.json` riavvia l'app.

### Calendari Google

Gli account Google configurati in Outlook come IMAP portano solo la posta, non il calendario.
Due modi per averlo comunque:

1. **Link iCal** (il più semplice): in Google Calendar, *Impostazioni → il calendario →
   Indirizzo segreto in formato iCal*; nell'app, **Gestisci calendari → Aggiungi un calendario
   da link**. Il link vale come una password e resta solo in `config.json` su questo PC.
2. **Lettura diretta via CalDAV**, se l'amministratore ha disattivato il link iCal: richiede
   l'add-in **CalDav Synchronizer** per Outlook, configurato una volta per quell'account
   Google. L'app riusa il suo token: basta aggiungere in `personalizzazione.json`
   `"calendari_google": [{"indirizzo": "tu@azienda.example", "nome": "Azienda"}]`.

Con i calendari letti da un server (sezione sotto) c'è un terzo modo, che non richiede né il link
né l'add-in: l'account Google si aggiunge nel telefono del server e il suo calendario arriva come
quelli di Outlook.

### Aziende inoltrate

Se un account si apre solo dal browser, un flusso (per esempio Power Automate) può copiarne le
riunioni nel calendario di un'altra casella con un prefisso nel titolo, come `Copia: Riunione`.
In `aziende_inoltrate` indichi la casella che le riceve (`via`), il prefisso (`prefisso`,
espressione regolare) e, se un altro flusso inoltra anche la posta, l'inizio dell'oggetto di
quelle mail (`oggetto_posta`). L'app mostra le copie come l'azienda d'origine, toglie il prefisso
dal titolo e usa la posta inoltrata per capire se il collegamento è ancora vivo.

### Caselle lette da un server

Se non vuoi (o non puoi) far leggere i calendari al PC, li legge un **agente** su un server
Linux e l'app glieli chiede. Il codice dell'agente è nella cartella `server/`. Per l'installazione
completa passo per passo (PC + server) e il perché delle scelte, vedi **[GUIDA.md](GUIDA.md)**.

Come funziona: sul server gira un telefono Android virtuale
([redroid](https://github.com/remote-android/redroid-doc), in Docker) con le app ufficiali,
**Outlook** per le caselle Microsoft e i **servizi Google** per i calendari Google, in cui si
fanno gli accessi una volta. Le app copiano gli appuntamenti nel calendario del telefono; l'agente legge
quel calendario e lo espone con una piccola API, in ascolto **solo sul localhost del server**.
L'app la raggiunge con un **tunnel SSH** che apre da sola: su internet non è esposto nulla e
l'accesso passa dalla tua chiave SSH, più una chiave dell'API.

Sul server (provato su Ubuntu ARM64; servono Docker e il modulo del kernel `binder_linux`):

1. copia `server/` in `/opt/oe-copilot`; crea `.env` da `.env.esempio` con una `API_KEY`
   lunga e casuale e `agent/fonti.json` da `agent/fonti.esempio.json` con le tue caselle;
2. `docker compose up -d` (costruisce l'immagine del telefono, con i servizi Google), poi installa
   nel telefono Outlook e fai gli accessi: in Outlook per le caselle Microsoft, in
   *Impostazioni → Password e account* per quelle Google (lo schermo si vede dal PC con
   [scrcpy](https://github.com/Genymobile/scrcpy) attraverso un tunnel SSH verso la porta 5555).
   In Outlook va acceso, casella per casella, *Impostazioni → Calendario → Sincronizza calendari*;
3. installa `strumenti/oe-copilot-sincronizza.service` come servizio systemd: tiene
   aggiornato il telefono ogni 30 secondi (da solo Android lo farebbe ogni 15 minuti).

Sul PC:

1. il server dev'essere raggiungibile con `ssh` **senza password** (chiave SSH);
2. salva la chiave dell'API, cifrata per il tuo utente Windows. Non passa mai dallo schermo:
   ```bat
   ssh utente@server "grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2" | .venv\Scripts\python agente_remoto.py imposta-chiave
   ```
3. in `personalizzazione.json` aggiungi `"agente_remoto": {"ssh": "utente@server"}` (o un
   alias di `~/.ssh/config`) e togli da `calendari_google` gli account che ora legge il
   server; riavvia l'app. `.venv\Scripts\python agente_remoto.py prova` dice se il server
   risponde.

Per tornare a Outlook basta togliere la voce `ssh`.

Le stesse caselle si possono vedere anche da un browser, senza app: il server serve la stessa
pagina dietro HTTPS e password (servizi `ui` e `caddy-ui`, vedi **GUIDA.md**, sezione 4.7).

Cosa cambia nell'app: il pallino di ogni azienda dice anche se sul server l'aggiornamento è
fermo; se il server non risponde l'app mostra l'ultima lettura salvata e riprova da sola;
*Apri in Outlook* cerca l'evento nell'Outlook del PC per ora e titolo. Limiti: gli appuntamenti
che esistono solo nell'Outlook del PC (calendari "solo questo computer") e i compleanni dei
contatti non ci sono, e le riunioni annullate spariscono invece di restare come "Annullato:".

Un accesso può scadere sul server (password cambiata, sessione revocata): il calendario sul
telefono resterebbe fermo e la lettura riuscirebbe lo stesso, col pallino verde su dati vecchi.
Per evitarlo `strumenti/rileva_accessi.py` (lanciato dal servizio di sincronizzazione) scrive in
`stato/accessi.json` lo stato di ogni casella, che l'agente usa per il pallino — senza mai
leggere un token:

- **Caselle Microsoft (Outlook):** la bandierina "login da rifare" di Outlook. Netta: quando
  c'è → pallino **rosso** "Accesso scaduto sul telefono".
- **Calendario Google (DAVx⁵):** DAVx⁵ non ha una bandierina simile e un login scaduto dà lo
  stesso errore di una rete assente. Ma il servizio lo spinge ogni 30 s e DAVx⁵ sincronizza da
  sé ogni 15 min: se la sua ultima sincronizzazione **riuscita** si ferma ben oltre (mentre le
  caselle Outlook vanno, quindi non è la rete) la causa è quasi sempre il login scaduto →
  **giallo** oltre 20 min, **rosso** oltre 45 min ("probabile accesso scaduto, ricollega
  DAVx⁵"). Se DAVx⁵ ha una notifica attiva che chiede di ricollegarsi, è rosso subito. Soglie
  regolabili con `DAVX5_GIALLO_S` / `DAVX5_ROSSO_S`.

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

Verifica codice, interfaccia, lettura delle caselle (da Outlook o dal server), coerenza dei
pallini di stato e che nessun dato
di `personalizzazione.json` sia finito nel codice. Con `--avvio` lancia anche l'app e controlla
che l'interfaccia parta davvero. Esce con codice 1 se qualcosa è rosso.

## File principali

| File | Cosa fa |
|------|---------|
| `app.py` | Finestra desktop e ponte con l'interfaccia |
| `config.py` | Impostazioni, colori, lettura di `personalizzazione.json` |
| `outlook_com.py` | Lettura di calendari ed eventi da Outlook (COM/MAPI) |
| `agente_remoto.py` | Lettura delle caselle da un server, attraverso un tunnel SSH |
| `caselle.py` | Regole comuni alle due letture: di che azienda è una riunione, doppioni, colori |
| `eventi.py` | Unione delle sorgenti, doppioni, stato di ogni azienda |
| `google_cal.py` | Calendari Google via CalDAV |
| `ics_feed.py` | Calendari da link iCal e festività italiane |
| `local_events.py` | Eventi aggiunti a mano nell'app |
| `reminders.py` | Promemoria: notifiche Windows e Telegram |
| `teams_link.py` | "Apri in Teams" sull'account giusto |
| `pulizia.py` | Sposta le riunioni finite nel calendario sbagliato (`pulizia.py "Azienda" [--applica]`) |
| `verifica.py` | Controllo di salute |
| `web/` | Interfaccia (FullCalendar) |
| `strumenti/` | Correzione di pywebview e relativa licenza |
| `server/` | L'agente che legge le caselle su un server (vedi *Caselle lette da un server*) |

Restano sul PC e fuori dal repository: `personalizzazione.json`, `config.json`, le cache
(`calendars_cache.json`, `events_cache.json`, `ics_cache/`, `agente_caselle.json`), la chiave
cifrata dell'agente (`agente_chiave.bin`), gli eventi personali, i log e la cartella
`personale/`.

## Se qualcosa non va

Apri **`debug.log`**: contiene gli avvii, i tempi di lettura e gli eventuali errori.

- **La finestra resta vuota**: di solito manca la connessione al primo avvio.
- **Non compare un calendario appena aggiunto in Outlook**: **Gestisci calendari → Ricerca
  calendari nuovi** (l'elenco è memorizzato perché esplorare tutte le caselle richiede tempo).
- **"Impossibile parlare con Outlook"**: apri Outlook classico e riprova.
- **"Il server dei calendari non risponde"** (caselle lette da un server): prova
  `.venv\Scripts\python agente_remoto.py prova`; il motivo di un tunnel che non parte è in
  `agente_tunnel.log`. Un pallino giallo o rosso su un'azienda dice nel suo suggerimento cosa
  è fermo sul server.
- **Outlook chiede "Scegli profilo"**: ci sono più profili e nessuno è predefinito. Indica quello
  da usare in `config.json` (`"outlook_profile": "NomeProfilo"`) o impostane uno predefinito in
  *Pannello di controllo → Posta → Mostra profili*.
