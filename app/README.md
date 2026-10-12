# OE Copilot per Windows

Client desktop dell'interfaccia di OE Copilot: una finestra (Tauri v2, WebView2) che mostra il sito vero, con la stessa sessione e gli stessi cookie del browser. Non contiene l'interfaccia: gli aggiornamenti del sito arrivano con il deploy del server, l'app cambia solo quando serve una funzione nativa.

Cosa aggiunge a una scheda del browser:
- **Icona nell'area di notifica** con il menu *Apri*, *Notifiche*, *Avvia con Windows*, *Esci*. Chiudere la finestra la nasconde, l'app resta attiva; *Esci* la chiude davvero.
- **Un solo processo.** Un secondo avvio riporta davanti la finestra già aperta.
- **Avvio con Windows**, spento di partenza, si accende dal menu. Parte nell'area di notifica (argomento `--nascosto`).
- **Notifiche di Windows per le riunioni in arrivo** (sotto).
- **Lista degli indirizzi ammessi**: nella finestra si carica solo il sito. Ogni altro link http o https si apre nel browser predefinito, gli altri schemi (`file:`, `javascript:`, `data:`...) sono rifiutati. Le pagine del sito aperte con `window.open` (il pulsante *Telefono*) si aprono in una finestra dell'app senza comandi né notifiche.
- **I link di Teams** (`teams.microsoft.com/l/meetup-join/...`, `teams.microsoft.com/meet/...`, `teams.live.com`) si aprono con lo schema `msteams:` (`msteams://teams.microsoft.com/l/...`, la forma indicata dalla documentazione di Teams), quindi nell'app Teams; se lo schema non è registrato si ripiega sul browser.
- **Pagina di avvio**: prima di caricare il sito l'app controlla che risponda. Se non risponde mostra un messaggio in italiano e riprova ogni 5 secondi, poi porta la finestra sul sito da sola. Se il sito cade mentre si usa l'app e una navigazione fallisce, la finestra torna a questa pagina invece di restare sulla pagina di errore del browser.
- **Dimensioni e posizione della finestra** ricordate tra un avvio e l'altro.

## Costruirla

Servono Node, Rust con il target `stable-x86_64-pc-windows-msvc`, gli strumenti di compilazione MSVC con il Windows SDK e il runtime WebView2 (già presente in Windows 11). Su questo PC l'ambiente è nello script `env-win.ps1` della cartella di prova di Tauri (variabili solo del processo, nessuna modifica permanente).

```powershell
. <percorso>\env-win.ps1                  # solo se l'ambiente non è già quello giusto
$env:OE_URL = 'https://oe.tuodominio.it'  # l'indirizzo del sito
cd app
npm ci
npm run build
```

Il risultato sta in `src-tauri/target/release/`: `oe-copilot-app.exe` e, in `bundle/nsis/`, l'installer `OE Copilot_<versione>_x64-setup.exe` (per utente, senza amministratore, non firmato: SmartScreen può chiedere conferma).

**`OE_URL`** è l'indirizzo del sito: schema, nome ed eventuale porta, senza percorso (`https://`; `http://` solo per `localhost`). Non sta in nessun file del repository: `scripts/configura.mjs` scrive `src-tauri/tauri.conf.local.json` (ignorato da git) con l'indirizzo al posto del segnaposto `https://oe.example.com` della configurazione tracciata, e `tauri build --config` lo usa. La build di rilascio si ferma con un errore se `OE_URL` manca o è ancora il segnaposto. Cambiare sito vuol dire ricostruire l'app.

La prima build compila anche le dipendenze e dura circa 6 minuti, le successive circa 3 (misurato su questo PC, 8 core). L'output di `src-tauri/target` pesa circa 4 GB ed è ignorato da git.

## Installare e usare

1. Eseguire l'installer: si installa per l'utente in `%LOCALAPPDATA%\OE Copilot`, senza amministratore, con i collegamenti nel menu Start e sul desktop. Si disinstalla dal pannello di Windows.
2. Aprire *OE Copilot* dal menu Start: la finestra mostra il sito, si entra con la solita password. Passkey e codici TOTP sono quelli del sito.
3. Dal menu dell'icona (a volte nascosta dietro la freccia dell'area di notifica) si accendono *Avvia con Windows* e *Notifiche*.

Le notifiche di Windows sono pensate per l'app installata: l'app le invia con l'identificativo `com.oecopilot.desktop`, che l'installer registra nel collegamento del menu Start. L'eseguibile lanciato dalla cartella di build non ha quella registrazione e le sue notifiche risultano di PowerShell.

Dati dell'app: `%APPDATA%\com.oecopilot.desktop` (impostazioni, posizione della finestra), `%LOCALAPPDATA%\com.oecopilot.desktop` (profilo WebView2 con i cookie). Disinstallando dal pannello di Windows va via l'app; le due cartelle restano finché non si cancellano.

Se l'app non parte o la finestra tarda ad aprirsi, i messaggi di errore stanno in `%LOCALAPPDATA%\com.oecopilot.desktop\errore.log` (con l'ora in secondi dal 1970). Riaprendola subito dopo una chiusura forzata WebView2 può risultare occupato per qualche secondo (errore 0x800700AA): l'app riprova fino a 10 volte, poi si ferma.

## Notifiche

Uno script incluso nell'app (`src-tauri/src/inizializzazione.js`) gira nella finestra principale, e solo sulla pagina del sito:
- ogni 60 secondi chiede `get_events` per le prossime 24 ore, con la stessa API e lo stesso cookie della pagina;
- per ogni riunione con orario che inizia entro 10 minuti manda una notifica di Windows con titolo, ora (di Roma) e luogo, senza il link per entrare; il click sulla notifica porta davanti la finestra;
- ogni avviso parte una volta sola per riunione e anticipo (l'elenco sta in `localStorage` e si ripulisce da solo); le giornate intere non avvisano mai; con i dati in cache perché l'agente non risponde non parte niente;
- senza sessione (risposta 401 o pagina di accesso) lo script smette di chiedere e riprova ogni 5 minuti; dopo un errore di rete allunga l'attesa fino a 5 minuti;
- l'interruttore *Notifiche* del menu li ferma subito e la scelta resta salvata;
- se il server espone `get_promemoria`, gli anticipi vengono da `minuti` (ogni 30 minuti si rilegge); `attivo: false` riguarda solo i promemoria del server, l'app avvisa comunque; senza quel metodo l'anticipo è la costante `MINUTI_PREAVVISO` (10) in testa allo script;
- nei log non finisce niente che riguardi gli eventi.

## Sicurezza

- I comandi nativi (IPC) sono concessi solo alla pagina del sito (`remote.urls` e `"local": false`, finestra `main`) con un elenco esplicito: `notifiche_attive` e tre permessi del plugin notifiche. Niente `core:default`. La pagina di avvio e le finestre in più non hanno nessun comando.
- Il plugin di apertura dei link non inietta il suo script (`open_js_links_on_click` spento): i link li gestisce la lista degli indirizzi ammessi.
- La finestra non ha strumenti per sviluppatori nelle build di rilascio.

## Cosa non c'è

- **Apri in Outlook** del vecchio client: non si può riprodurre, gli id degli eventi del sito non sono id di Outlook.
- **Aggiornamento automatico**: serve un indirizzo pubblico per i file, il repository è privato. Si reinstalla a mano.
- **Firma del codice** dell'installer.
- **Android**: pacchetto successivo.
- **Uso offline**: se il sito non risponde c'è solo il messaggio della pagina di avvio, nessuna copia del calendario.

## Prove

- `npm test` (in `app/`, solo Node) prova lo script delle notifiche con una pagina finta: ora di Roma, un avviso solo per riunione, esclusioni, sessione finita, attese dopo gli errori.
- `cargo test` (in `src-tauri/`) prova la lista degli indirizzi ammessi e il passaggio dei link di Teams a `msteams:`.
- Il resto (finestra, accesso, notifiche vere, icona, istanza unica) si prova con una build `--features prova` contro un gateway locale: la feature aggiunge solo un comando che legge e gira gli interruttori del menu e, con la variabile `OE_PROVA_APRI`, fa scrivere su un file gli indirizzi destinati al sistema invece di aprirli. Non fa parte di nessuna build di rilascio.
