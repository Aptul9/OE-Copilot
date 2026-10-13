# OE Copilot per Windows e per Android

Client dell'interfaccia di OE Copilot: una finestra (Tauri v2, WebView2 su Windows) che mostra il sito vero, con la stessa sessione e gli stessi cookie del browser. Esiste per Windows e per Android (sezione *Android*, più sotto). Non contiene l'interfaccia: gli aggiornamenti del sito arrivano con il deploy del server, l'app cambia solo quando serve una funzione nativa.

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

La prima build compila anche le dipendenze e dura circa 5 minuti, le successive da 2 a 6 secondo quanto è cambiato (misurato su questo PC, 8 core). L'output di `src-tauri/target` pesa circa 4 GB ed è ignorato da git.

## Installare e usare

1. Eseguire l'installer: si installa per l'utente in `%LOCALAPPDATA%\OE Copilot`, senza amministratore, con i collegamenti nel menu Start e sul desktop. Si disinstalla dal pannello di Windows.
2. Aprire *OE Copilot* dal menu Start: la finestra mostra il sito, si entra con la solita password. Passkey e codici TOTP sono quelli del sito.
3. Dal menu dell'icona si accendono *Avvia con Windows* e *Notifiche*. L'icona nuova sta dietro la freccia dell'area di notifica (*Mostra icone nascoste*), si può trascinare nella barra. Un clic sinistro porta davanti la finestra (non la nasconde), il tasto destro apre il menu.

Le notifiche di Windows sono pensate per l'app installata: l'app le invia con l'identificativo `com.oecopilot.desktop`, che l'installer registra nel collegamento del menu Start. L'eseguibile lanciato dalla cartella di build non ha quella registrazione e le sue notifiche risultano di PowerShell. L'avviso compare come banner in basso a destra e resta nel centro notifiche; un clic sul banner porta davanti la finestra, anche se era nascosta nell'area di notifica. Con *Non disturbare*, con un'app a schermo intero o a sessione bloccata resta solo il centro notifiche. Nelle prove sul PC il banner è comparso in 12 avvii su 14: negli altri 2 l'avviso era nel centro notifiche ma senza banner, la causa non è stata trovata.

Dati dell'app: `%APPDATA%\com.oecopilot.desktop` (impostazioni, posizione della finestra), `%LOCALAPPDATA%\com.oecopilot.desktop` (profilo WebView2 con i cookie). Disinstallando dal pannello di Windows va via l'app; le due cartelle restano finché non si cancellano, e anche la chiave `HKCU\Software\oecopilot\OE Copilot` (il percorso di installazione, lasciata dal disinstallatore di Tauri), innocua.

Se l'app non parte o la finestra tarda ad aprirsi, i messaggi di errore stanno in `%LOCALAPPDATA%\<identificativo>\errore.log`, cioè `com.oecopilot.desktop` (con l'ora in secondi dal 1970); una build di prova con un altro identificativo scrive nella sua cartella. Riaprendola subito dopo una chiusura forzata WebView2 può risultare occupato per qualche secondo (errore 0x800700AA): l'app riprova fino a 10 volte, poi si ferma.

## Accesso e sessione

Il login si fa una volta e resta salvato: il sito dà un cookie di sessione (`__Secure-better-auth.session_token`) che dura 30 giorni, ed è un cookie persistente (`Max-Age`), quindi WebView2 lo scrive nel profilo in `%LOCALAPPDATA%\com.oecopilot.desktop`. Chiudere la finestra, uscire dall'app o riavviare Windows non lo toglie: alla riapertura la finestra mostra subito il calendario. La password non è salvata da nessuna parte: nell'app resta solo il cookie.

Se la sessione non vale più, l'app torna da sola alla pagina di accesso, senza fare niente di speciale: è il sito a deciderlo.
- le pagine senza sessione rimandano a `/login?next=...`;
- una risposta 401 dell'API (per esempio dopo che la sessione è stata chiusa) riporta la pagina a `/login`;
- le sessioni si chiudono quando cambia `ADMIN_PASSWORD` (nuovo secret e nuovo aggiornamento, GUIDA.md 4.7), quando l'utente viene tolto da `gateway.db` o dopo i 30 giorni.

Si deve rifare il login anche se si cancellano i dati dell'app (la cartella `%LOCALAPPDATA%\com.oecopilot.desktop`) o i cookie di WebView2.

**Trappola: l'installer va eseguito da Windows, non da dentro un'app impacchettata (MSIX), per esempio un terminale o un assistente distribuiti come pacchetto.** Un'app impacchettata rimanda in una cartella sua (`%LOCALAPPDATA%\Packages\<pacchetto>_...\LocalCache\Local`) tutto ciò che un programma lanciato da lei scrive in `%LOCALAPPDATA%` e `%APPDATA%`. Un'app installata o avviata così ha programma, profilo e cookie in quella cartella, e ne esiste un'altra copia "vera" per chi la lancia dal desktop o dal menu Start: due profili, due login, e il login "sparisce" a seconda dell'icona con cui si apre. Sintomo: ogni avvio chiede di nuovo la password, e `session` in `ui-dati/gateway.db` ha una riga nuova a ogni avvio (user agent `Mozilla/5.0 (Windows NT 10.0...`). Rimedio: disinstallare, cancellare la cartella virtuale `OE Copilot` e `com.oecopilot.desktop` dentro `Packages\<pacchetto>_...\LocalCache\Local`, e reinstallare con un doppio clic sull'installer. Da uno strumento che gira dentro un'app impacchettata si esce dal pacchetto creando il processo con WMI: `Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = '"<installer>" /S' }`; le icone create puntano così a `C:\Users\<utente>\AppData\Local\OE Copilot`.

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

## Android (costruita e provata su un emulatore)

Lo stesso client per Android (pacchetto `com.oecopilot.app`): la finestra del sistema (WebView) sul sito, con le notifiche del plugin e la lista degli indirizzi ammessi come sul PC. È costruita, firmata e provata su un emulatore (Android 15, WebView 124, x86_64). Su un telefono vero non è mai stata provata.

### Cosa fa

- Mostra il sito con lo stesso accesso del browser: il cookie di sessione dura 30 giorni e la WebView lo scrive su disco quando l'app passa in secondo piano (altrimenti ogni 30 secondi circa, e un login seguito subito da una chiusura andrebbe perso). La pagina sta fra la barra di stato e quella di navigazione e si accorcia quando compare la tastiera.
- Lista degli indirizzi ammessi: un link a un altro sito (anche `target=_blank` e `window.open`) si apre nel browser e la pagina resta dov'è; `file:`, `data:` e `about:blank` sono rifiutati. Una pagina del sito aperta con `window.open` (il pulsante *Telefono*) prende il posto della pagina corrente, perché la vista è una sola: il tasto indietro riporta al calendario.
- Notifiche per le riunioni con orario che iniziano entro 10 minuti: lo stesso script del PC (sezione *Notifiche*). Da Android 13 il permesso `POST_NOTIFICATIONS` si chiede una volta, alla prima verifica dopo l'accesso (finestra di sistema *Consenti*); se si rifiuta non parte niente. Lo script gira solo mentre il processo dell'app è vivo: dopo il tasto indietro dalla prima pagina, o con l'app tolta dalle recenti, non parte niente. I promemoria con l'app chiusa li manda il server su Telegram (GUIDA.md, 4.9).
- Se il sito non risponde si vede la pagina di avvio con il messaggio in italiano e il pulsante *Riprova ora*; l'app riprova ogni 5 secondi e porta la vista sul sito da sola.
- Non ci sono le notifiche del web, le passkey (la WebView di Android le espone solo se l'app lo chiede e il sito pubblica gli Asset Links) né i comandi del PC (icona nell'area di notifica, istanza unica, avvio con Windows).

### Costruirla

Servono JDK 17, l'SDK di Android con la piattaforma 37 e gli strumenti di compilazione 37.0.0, l'NDK 29.0.14206865, Rust con i target `aarch64-linux-android` e `x86_64-linux-android`, e le variabili `JAVA_HOME`, `ANDROID_HOME` e `NDK_HOME`. Su questo PC le imposta `env-android.ps1`, accanto a `env-win.ps1` nella cartella di prova di Tauri (solo variabili del processo).

```powershell
. <percorso>\env-android.ps1               # solo se l'ambiente non è già quello giusto
$env:OE_URL = 'https://oe.tuodominio.it'   # l'indirizzo del sito
cd app
npm ci --ignore-scripts
npm run android:build
```

Escono due APK sotto `src-tauri/gen/android/app/build/outputs/apk/`: `arm64/release/app-arm64-release.apk` per i telefoni (arm64-v8a, circa 7,5 MiB) e `x86_64/release/app-x86_64-release.apk` per l'emulatore (circa 8,0 MiB). `OE_URL` è come per il PC: senza, la build si ferma; l'indirizzo sta nell'APK e non in nessun file del repository.

La prima volta serve circa un quarto d'ora (Gradle scarica sé stesso e le dipendenze, Rust compila due architetture), le volte dopo da 4 a 7 minuti (misurato su questo PC, 8 core). Gradle usa circa 3 GB di memoria e cargo circa 2 GB: conviene chiudere il browser. Alla fine `gradlew --stop`, nella cartella `src-tauri/gen/android`, ferma il demone di Gradle.

### Firma

Il rilascio si firma con un keystore che sta FUORI dal repository, in `%USERPROFILE%\.oe-copilot\android\` (file `keystore.properties` con `storeFile`, `storePassword`, `keyAlias`, `keyPassword`; un altro percorso con la variabile `OE_KEYSTORE_PROPERTIES`). Le chiavi sono anche nella nota `_secrets/oe-copilot` del vault. Senza il file l'APK esce non firmato e non si installa. Le firme sono v2 e v3 (v1 non serve, minSdk 24).

L'impronta SHA-256 del certificato è `47:F3:FD:35:FD:C8:0E:9E:D6:15:93:90:05:2B:8F:E3:84:24:25:20:69:AB:15:F3:6A:09:D5:1B:90:FB:31:F5` e si controlla con `apksigner verify --print-certs <apk>`. Un APK firmato con un'altra chiave non aggiorna quello già installato (`INSTALL_FAILED_UPDATE_INCOMPATIBLE`, provato): la chiave non si rigenera.

### Installarla

- A mano: si copia l'APK sul telefono e lo si apre; Android chiede di consentire l'installazione da quella fonte (*origini sconosciute*) e Play Protect avvisa perché lo sviluppatore non è nello store.
- Con adb, telefono collegato in debug USB: `adb install OE-Copilot-0.1.0-arm64-v8a.apk`. Per aggiornare: `adb install -r` con l'APK nuovo, firmato con la stessa chiave.
- Il sito si apre da solo; l'accesso è quello del sito (password, passkey e TOTP del sito). Alla prima notifica Android chiede il permesso: va concesso.

### Provarla sull'emulatore

`npm run android:prova` costruisce una variante di debug con un altro pacchetto, `com.oecopilot.app.prova` (il suffisso viene da `bundle.android.debugApplicationIdSuffix` in `tauri.android.conf.json`: la CLI di Tauri riscrive la riga `applicationIdSuffix` di `build.gradle.kts` a ogni build, una riga messa a mano sparisce), che si installa accanto all'app firmata e accetta il traffico in chiaro solo verso `10.0.2.2`, il PC visto dall'emulatore (`src/debug/res/xml/network_security_config.xml`; verso altri indirizzi la WebView risponde `ERR_CLEARTEXT_NOT_PERMITTED`). Si usa con `OE_URL=http://10.0.2.2:<porta>` e un gateway di prova in locale.

Solo la variante di prova espone il socket di DevTools: `adb forward tcp:<porta> localabstract:webview_devtools_remote_<pid dell'app>` (il pid si trova in `/proc/net/unix`) e poi si guida la pagina come un Chrome senza finestra. L'emulatore parte con `emulator -avd <nome> -no-window -no-audio -no-snapshot -gpu angle_indirect -dns-server 8.8.8.8,1.1.1.1` (senza `-dns-server` la WebView non risolve i nomi). `uiautomator dump` non vede il testo della WebView: si usano schermate e DevTools.

### Trappole

- La WebView di Android lancia lo script di inizializzazione due volte sulle pagine del sito (all'avvio del documento e a pagina iniziata): lo script lo ignora la seconda volta (`window.__oeInizializzato`), senza questo i giri sono doppi e il permesso si chiede due volte.
- Le icone di `gen/android/app/src/main/res` vanno copiate da `src-tauri/icons/android` (le fa `tauri icon` solo se il progetto Android esiste già); quelle del modello sono il logo di Tauri. Il primo piano dell'icona adattiva passa da `drawable/ic_launcher_foreground_inset.xml` (margine del 20%): il disegno arriva agli angoli e la maschera rotonda del telefono lo taglierebbe. Rifare `tauri icon` riscrive `mipmap-anydpi-v26/ic_launcher.xml` e toglie il margine.
- Il plugin delle notifiche aggiunge `RECEIVE_BOOT_COMPLETED` e `WAKE_LOCK`, per le notifiche programmate che l'app non usa: `AndroidManifest.xml` li toglie, restano `INTERNET` e `POST_NOTIFICATIONS`.
- La CLI avvisa che l'identificativo `com.oecopilot.app` finisce per `.app`: vale solo per macOS, qui non conta.

## Cosa non c'è

- **Apri in Outlook** del vecchio client: non si può riprodurre, gli id degli eventi del sito non sono id di Outlook.
- **Aggiornamento automatico**: serve un indirizzo pubblico per i file, il repository è privato. Si reinstalla a mano.
- **Firma del codice** dell'installer.
- **Pulsanti nelle notifiche** (*Partecipa*) e avviso in stile Teams (finestra propria, sempre in primo piano): non ci sono. La notifica è quella normale del sistema, con titolo, ora e luogo.
- **Android**: provata solo su un emulatore, mai su un telefono vero; senza notifiche del web né passkey (sezione sopra).
- **Uso offline**: se il sito non risponde c'è solo il messaggio della pagina di avvio, nessuna copia del calendario.

## Prove

- `npm test` (in `app/`, solo Node) prova lo script delle notifiche con una pagina finta: ora di Roma, un avviso solo per riunione, esclusioni, sessione finita, attese dopo gli errori.
- `cargo test` (in `src-tauri/`) prova la lista degli indirizzi ammessi e il passaggio dei link di Teams a `msteams:`. Gira sul PC: la regola che su Android rifiuta `about:blank` si prova solo sull'emulatore.
- Android: la variante di prova contro un gateway locale (con un agente dei dati inventati) ha provato accesso, calendario, permesso e avviso di notifica (uno solo, senza ripetizioni dopo il ricaricamento e il riavvio), sessione che resta dopo la chiusura forzata, uscita che ferma i giri, link esterni, `file:`, `data:` e `about:blank`, sito non raggiungibile e ritorno da solo, tasto indietro. L'APK di rilascio è provato in installazione, avvio sul sito vero (schermata di accesso, niente digitato), aggiornamento con la stessa chiave e rifiuto con un'altra.
- Il resto (finestra, accesso, notifiche, icona, istanza unica, link) si prova con una build `--features prova` contro un gateway locale: la feature aggiunge solo un comando che legge e gira gli interruttori del menu e, con la variabile `OE_PROVA_APRI`, fa scrivere su un file gli indirizzi destinati al sistema invece di aprirli. Non fa parte di nessuna build di rilascio. Una build di prova deve avere un altro nome e un altro identificativo, così non tocca dati, istanza unica, avvio con Windows e collegamenti di un'installazione vera: `node scripts/configura.mjs --prova`, poi `npx tauri build --no-bundle --features prova --config src-tauri/tauri.conf.local.json --config prova.json`, con in `prova.json` `{ "productName": "OE Copilot Prova", "identifier": "com.oecopilot.desktop.prova" }` (la seconda `--config` si fonde sopra la prima).
- Banner e clic sulle notifiche, uso dell'icona e consegna reale dei link al browser si provano con la stessa build senza la feature, installata con l'installer (`/S /D=<cartella>`, nome e identificativo di prova): Windows mostra i banner solo per un'app installata e la feature cambierebbe il percorso dei link. Il banner si guarda con una foto del solo angolo dei toast, il clic sul banner con un clic verificato un attimo prima, l'icona con UI Automation (Invoke sul pulsante; il menu si apre con Maiusc+F10 sul pulsante selezionato e le sue voci si scorrono con le frecce). Il browser predefinito per https è quello di `HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoiceLatest\ProgId`: su Windows 11 recente la vecchia chiave `UserChoice` può indicarne un altro. Il passaggio di un link a Teams con `msteams:` non è provato con un Teams vero (su quel PC era aperto): la traduzione la provano `cargo test` e la build di prova.
