# OE Copilot per Windows e per Android

Client di OE Copilot: una finestra Tauri v2 (WebView2 su Windows, WebView su Android) sul sito vero, con la sua sessione e i promemoria delle riunioni. L'interfaccia sta nel sito; l'app cambia solo per le funzioni native.

- Nella finestra si carica solo il sito: gli altri link http e https vanno al browser, gli altri schemi sono rifiutati. Se il sito non risponde, una pagina di avvio riprova ogni 5 secondi.
- Su Windows: icona nell'area di notifica con *Apri*, *Avvia con Windows* ed *Esci* (chiudere la finestra la nasconde), un solo processo, link di Teams aperti con lo schema `msteams:`. *Avvia con Windows* passa `--nascosto`: l'app parte nell'area di notifica, senza prendere il fuoco.

## Costruirla (Windows)

Servono Node, Rust con il target `stable-x86_64-pc-windows-msvc`, MSVC con il Windows SDK e WebView2.

```powershell
. <percorso>\env-win.ps1                  # solo se l'ambiente non è già quello giusto
$env:OE_URL = 'https://<sito>'            # l'indirizzo del sito
cd app
npm ci
node scripts/configura.mjs --release
npx tauri build --no-bundle --config src-tauri/tauri.conf.local.json
```
`npm run build` fa gli stessi due passi. Esce `app/src-tauri/target/release/oe-copilot-app.exe`.

**Trappola: l'indirizzo del sito sta dentro l'exe.** `scripts/configura.mjs` scrive `OE_URL` (schema, nome ed eventuale porta; `http://` solo per `localhost`) in `src-tauri/tauri.conf.local.json`, ignorato da git, al posto del segnaposto `https://oe.example.com`. Con `--release` il segnaposto ferma la build. Cambiare sito vuol dire ricostruire.

## Installare e aggiornare (Windows)

Un file solo, senza installer: l'exe copiato come `%LOCALAPPDATA%\Programs\OE Copilot\OE Copilot.exe`, nient'altro nella cartella, e un collegamento *OE Copilot* nel menu Start. Per aggiornare: *Esci* dal menu dell'icona, l'exe nuovo sopra il vecchio, riavvio dal menu Start.

Login e profilo di WebView2 stanno in `%LOCALAPPDATA%\com.oecopilot.desktop`, legati all'identificativo e non al percorso: sostituire l'exe lascia il login. Gli errori di avvio vanno in `errore.log`, nella stessa cartella.

## Accesso e sessione

Il cookie di sessione dura 30 giorni e resta nel profilo; la password non è salvata.

**Trappola: copiare e avviare l'exe da Windows**, non da un terminale o un assistente distribuiti come app impacchettata (MSIX). Quello che un programma lanciato da lì scrive in `%LOCALAPPDATA%` finisce in `%LOCALAPPDATA%\Packages\<pacchetto>_...\LocalCache\Local`: secondo profilo, password chiesta a ogni avvio. Dal pacchetto si esce con WMI: `Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = '<comando>' }`.

## Promemoria

Si scelgono nel sito, *Impostazioni > Promemoria* ([README, 4.9](../README.md#49-i-promemoria)). Quando avvisare e cosa scrivere lo decide il nucleo in Rust `src-tauri/src/promemoria.rs`, uguale per Windows e Android: dalle impostazioni e dagli eventi dell'API una voce per riunione e anticipo, con l'ora di Roma, il titolo e il corpo. Un avviso per riunione e anticipo; niente per i giorni interi o con dati in cache. Le WebView non hanno notifiche web: nell'app manca *Notifiche del browser*.

Su Windows lo script `src-tauri/src/inizializzazione.js`, sulla pagina del sito, ogni minuto rilegge impostazioni ed eventi e passa al nucleo (comando `pianifica_promemoria`); gli avvisi partono solo ad app in esecuzione, e un clic porta davanti la finestra. Su Android vedi sotto.

Su Windows il sito mostra *Notifica di Windows* e *Avviso a comparsa*: una finestra dell'app (`src-tauri/src/comparsa.rs`) in basso a sinistra, visibile anche con *Non disturbare*, con una riga per riunione (al più 6) che sparisce dopo 5 secondi. Con *Solo ora e azienda* mostra logo e ora. Il suono scelto lo fa l'avviso (l'evento di Windows della notifica con quel nome, con `PlaySoundW`), e allora la notifica è muta; ad avviso spento lo fa la notifica. La notifica la fa l'app (`src-tauri/src/notifica.rs`, non il plugin, che su Windows passa solo titolo, testo e suono): una per riunione (quella dei 10 minuti prende il posto di quella dei 30), esce dal centro notifiche all'inizio della riunione e ha il logo dell'azienda, salvato in `%LOCALAPPDATA%\com.oecopilot.desktop\loghi`. Se non riesce, la fa il plugin.

**Trappola: il banner.** Windows mostra il banner solo per un identificativo che conosce, quindi a ogni avvio l'app scrive `HKCU\Software\Classes\AppUserModelId\com.oecopilot.desktop`; senza, l'avviso resta nel centro notifiche. Un exe lanciato da `target\debug` o `target\release` notifica come PowerShell (regola del plugin): il banner si prova da un'altra cartella.

## Sicurezza

I comandi nativi valgono solo per la pagina del sito (`remote.urls`, `"local": false`, finestra `main`), con un elenco esplicito: `scripts/configura.mjs` rifiuta ogni permesso `:default`. Pagina di avvio, finestre in più e avviso a comparsa non hanno comandi, e le build di rilascio non hanno DevTools.

## Android

Pacchetto `com.oecopilot.app`; `file:`, `data:` e `about:blank` sono rifiutati. Niente notifiche web, niente passkey.

### Promemoria ad app chiusa

Le notifiche ci sono sempre; il permesso `POST_NOTIFICATIONS` lo chiede la pagina una volta, dopo l'accesso. Il resto lo fa l'app, in `gen/android/app/src/main/java/com/oecopilot/app/Promemoria.kt`:
- un lavoro di WorkManager ogni 15 minuti, e subito quando l'app va in secondo piano (dopo l'accesso, o dopo un cambio delle impostazioni sul sito), legge `get_promemoria` e `get_events` con il cookie della WebView, all'indirizzo di `plugins.oe.url` in `assets/tauri.conf.json`;
- il nucleo (funzione JNI `NucleoPromemoria.aggiorna`) dice quali allarmi programmare, quali avvisi mostrare subito (anticipo già passato, riunione non ancora iniziata, mai vista) e quali allarmi annullare;
- gli allarmi sono esatti (`USE_EXACT_ALARM`) e partono anche ad app chiusa e con il telefono in Doze; la notifica è una per riunione (quella dei 10 minuti prende il posto di quella dei 30), sparisce all'inizio e un tocco apre l'app;
- la notifica ha il logo dell'azienda in un disco bianco, o la sua sigla nel suo colore, come l'avviso a comparsa del PC: sigle e loghi vengono da `get_personalizzazione`, e il logo si scarica alla lettura (in `cache/loghi`, uno per indirizzo), perché l'allarme può partire senza rete. Nella barra di stato la sagoma bianca dell'icona (`drawable-*/ic_stat_promemoria.png`, da `scripts/genera_icona.py`): Android lì usa solo il canale alfa;
- senza sessione gli allarmi si annullano; con dati in cache restano quelli che ci sono.

Per un lavoro in orario anche a telefono fermo: *Impostazioni > App > OE Copilot > Batteria > Senza restrizioni*. Senza, gli allarmi partono lo stesso, ma una riunione aggiunta o cancellata può arrivare tardi. Dopo un riavvio gli allarmi tornano al primo giro del lavoro. Finché l'ottimizzazione della batteria è attiva, una fascia rossa in cima al calendario lo ricorda: l'app lo dice alla pagina (`Batteria.kt`, `window.OeBatteria`), il tasto **Disattiva** apre la richiesta di Android, e al ritorno nell'app la fascia se ne va. Su HyperOS *Nessuna restrizione* nella batteria dell'app vale come ottimizzazione tolta.

### Costruirla

Servono JDK 17, l'SDK di Android (piattaforma 37, strumenti 37.0.0), l'NDK 29.0.14206865, Rust con `aarch64-linux-android` e `x86_64-linux-android`, e le variabili `JAVA_HOME`, `ANDROID_HOME`, `NDK_HOME`.

```powershell
. <percorso>\env-android.ps1               # solo se l'ambiente non è già quello giusto
$env:OE_URL = 'https://<sito>'             # l'indirizzo del sito
cd app
npm ci --ignore-scripts
npm run android:build
```
Sotto `src-tauri/gen/android/app/build/outputs/apk/` escono `app-arm64-release.apk` per i telefoni e `app-x86_64-release.apk` per l'emulatore.

### Firma

Il rilascio si firma con `%USERPROFILE%\.oe-copilot\android\keystore.properties` (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`; altrove con `OE_KEYSTORE_PROPERTIES`), fuori dal repository, con copia nella nota `_secrets/oe-copilot` del vault. Senza, l'APK resta non firmato e non si installa.

**Trappola: la chiave non si rigenera.** Un APK con un'altra chiave non aggiorna quello installato (`INSTALL_FAILED_UPDATE_INCOMPATIBLE`). Impronta SHA-256 giusta, da `apksigner verify --print-certs <apk>`: `47:F3:FD:35:FD:C8:0E:9E:D6:15:93:90:05:2B:8F:E3:84:24:25:20:69:AB:15:F3:6A:09:D5:1B:90:FB:31:F5`. Si aggiorna con `adb install -r <apk>`.

### Trappole

- `npm run android:prova` fa `com.oecopilot.app.prova`, che convive con l'app firmata e parla in chiaro solo con `10.0.2.2`, il PC visto dall'emulatore. Il suffisso sta in `bundle.android.debugApplicationIdSuffix` di `src-tauri/tauri.android.conf.json`: la CLI riscrive `build.gradle.kts` a ogni build.
- L'emulatore va avviato con `-dns-server 8.8.8.8,1.1.1.1`, altrimenti la WebView non risolve i nomi.
- `tauri icon` riscrive `mipmap-anydpi-v26/ic_launcher.xml` e perde il margine di `drawable/ic_launcher_foreground_inset.xml`.
- *Forza interruzione* (o `adb shell am force-stop`) cancella allarmi e lavoro dei promemoria finché l'app non si riapre. Per provare l'app chiusa: `adb shell am kill <pacchetto>` con l'app in secondo piano.

## Cosa non c'è

Aggiornamento automatico (il repository è privato), firma del codice, pulsanti nelle notifiche. Senza rete si vede l'ultima copia del sito (README, 4.11).

## Prove

- `cargo test` (in `src-tauri/`) prova il nucleo dei promemoria (ora di Roma, anticipi, testi, allarmi di Android), indirizzi ammessi, link di Teams e righe dell'avviso; `npm test` (in `app/`) lo script della pagina, con il nucleo finto. La Action non prova `app/`.
- Una build di prova accanto a quella vera vuole un altro identificativo, con una seconda `--config` (`{ "productName": "OE Copilot Prova", "identifier": "com.oecopilot.desktop.prova" }`): con lo stesso condivide profilo, istanza unica e avvio con Windows.
