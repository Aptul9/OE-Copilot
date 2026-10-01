# OE Copilot

App desktop per Windows che **unisce in un calendario solo i calendari di tutti gli account
configurati in Outlook** (aziendali e personali), con un colore per azienda, e manda i
**promemoria delle riunioni** come notifica di Windows e, se vuoi, su **Telegram**.
Ha anche una sezione **Timesheet** con le schede dei gestionali delle aziende.

L'app legge quello che Outlook desktop ha già sul PC: nessun login, nessuna password, nessuna
registrazione su Azure. I dati restano sul PC.

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

### Aziende inoltrate

Se un account si apre solo dal browser, un flusso (per esempio Power Automate) può copiarne le
riunioni nel calendario di un'altra casella con un prefisso nel titolo, come `Copia: Riunione`.
In `aziende_inoltrate` indichi la casella che le riceve (`via`), il prefisso (`prefisso`,
espressione regolare) e, se un altro flusso inoltra anche la posta, l'inizio dell'oggetto di
quelle mail (`oggetto_posta`). L'app mostra le copie come l'azienda d'origine, toglie il prefisso
dal titolo e usa la posta inoltrata per capire se il collegamento è ancora vivo.

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

Verifica codice, interfaccia, lettura da Outlook, coerenza dei pallini di stato e che nessun dato
di `personalizzazione.json` sia finito nel codice. Con `--avvio` lancia anche l'app e controlla
che l'interfaccia parta davvero. Esce con codice 1 se qualcosa è rosso.

## File principali

| File | Cosa fa |
|------|---------|
| `app.py` | Finestra desktop e ponte con l'interfaccia |
| `config.py` | Impostazioni, colori, lettura di `personalizzazione.json` |
| `outlook_com.py` | Lettura di calendari ed eventi da Outlook (COM/MAPI) |
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

Restano sul PC e fuori dal repository: `personalizzazione.json`, `config.json`, le cache
(`calendars_cache.json`, `events_cache.json`, `ics_cache/`), gli eventi personali, i log e la
cartella `personale/`.

## Se qualcosa non va

Apri **`debug.log`**: contiene gli avvii, i tempi di lettura e gli eventuali errori.

- **La finestra resta vuota**: di solito manca la connessione al primo avvio.
- **Non compare un calendario appena aggiunto in Outlook**: **Gestisci calendari → Ricerca
  calendari nuovi** (l'elenco è memorizzato perché esplorare tutte le caselle richiede tempo).
- **"Impossibile parlare con Outlook"**: apri Outlook classico e riprova.
- **Outlook chiede "Scegli profilo"**: ci sono più profili e nessuno è predefinito. Indica quello
  da usare in `config.json` (`"outlook_profile": "NomeProfilo"`) o impostane uno predefinito in
  *Pannello di controllo → Posta → Mostra profili*.
