# Origine e modifiche

Copia di **ws-scrcpy** (https://github.com/NetrisTV/ws-scrcpy, licenza MIT, file `LICENSE`), potata e aggiornata per un solo uso: mostrare e comandare il telefono virtuale di OE Copilot da una pagina del browser.

| | |
|---|---|
| Commit copiato | `cd6cea6ebde17af6685c2b325183fc24bef9d52b` (2026-09-30, `0.9.0-dev`, Merge pull request #414) |
| Server sul telefono | `vendor/Genymobile/scrcpy/scrcpy-server.jar`, fork WebSocket di scrcpy 1.19 (NetrisTV/scrcpy, ramo `feature/websocket-v1.19.x`, versione `1.19-ws8`), licenza Apache-2.0 in `vendor/Genymobile/scrcpy/LICENSE` |
| Impronta del file | `752773d862915f3c6f8a14419aa7ea91b8778c16d26eac7bfeb2acd49fe4b17b` (`vendor/Genymobile/scrcpy/scrcpy-server.jar.sha256`, la prova di fumo la confronta con il file nel pacchetto) |

La prima commit della cartella è la copia così com'è (solo i fine riga portati a LF); le commit dopo mostrano ogni modifica.

## Tolto

- iOS: `appl-device`, `applDevice`, Appium e il suo script di installazione, `ios-device-lib`, `node-mjpeg-proxy`, i lettori MJPEG e QVH.
- Shell remota: `RemoteShell`, `ShellClient`, `node-pty`, `xterm` e i suoi componenti (`node-pty` compilava codice nativo).
- Web view (devtools), elenco dei file, caricamento dei file con il trascinamento (`FileListing`, `filePush`, `ExtendedSync`).
- Lettori software `Broadway` e `TinyH264` (WebAssembly, `worker-loader`): Chrome, Edge, Safari 16.4 e Firefox 130 hanno WebCodecs, e per gli altri resta il lettore MSE (`h264-converter`).
- Host remoti (`HostTracker`), file di configurazione YAML e HTTPS nel servizio (davanti c'è Caddy), indirizzo `proxy-ws` verso un indirizzo qualunque, tunnel verso una porta qualunque del telefono.
- `eslint`, `prettier`, `ts-node`, `tslib`, `yaml`, `file-loader` di `.asset`, e i file di esempio e la documentazione di upstream.

## Cambiato

- `proxy-adb` accetta solo `remote=tcp:8886` (il server di scrcpy sul telefono): la porta di adb o altre non si raggiungono dal browser.
- Il server sul telefono ascolta solo su localhost (`SCRCPY_LISTENS_ON_ALL_INTERFACES` spento): ci si arriva soltanto con `adb forward`.
- La pagina collega il flusso solo allo stesso sito della pagina: un indirizzo con `ws=`, `hostname=` o `port=` di un altro sito viene rifiutato.
- Qualità di partenza: 15 fotogrammi al secondo, 1 Mbit/s, 720x1280 (`src/app/player/StreamDefaults.ts`; numeri nella decisione 2026-10-11-oe-copilot-phone-browser del vault).
- Un solo telefono: `entrypoint.sh` collega adb a `TELEFONO` (`telefono:5555`) e lo riprova ogni 15 secondi.
- Solo HTTP, porta `PORT` (8000), `/salute` per il controllo di Docker.
- Aiuto: pulsante `?` in cima alla barra (`src/app/googDevice/toolbox/GoogHelpBox.ts`, `src/style/help.css`, icona `ic_help_678_48dp.svg`): pannello fisso con mouse, tastiera, pulsanti e sblocco, tasti evidenziati con `kbd`. Va aggiornato insieme a `KeyText.ts` e alla barra.
- Tastiera: collegata al telefono da subito (`captureKeyboard` vale `true` se manca). I caratteri partono come testo secondo `event.key` (`src/app/googDevice/KeyText.ts`, provato in `test/tasti.test.mjs`), non come codice del tasto fisico: con la tastiera italiana il codice faceva arrivare `-` come `/`, `_` come `?` e perdeva `@`. Ctrl, Alt e F1..F12 restano al browser, Ctrl+V manda il testo degli appunti del computer (evento `paste`), i campi della pagina (la casella di `More`) scrivono per conto loro, Invio, Backspace, Tab, Esc e le frecce restano codici Android. `TextControlMessage` scrive la lunghezza in byte UTF-8, non in caratteri.
- Strumenti: TypeScript 6, `@types/node` 26, `@types/ws` 8, Express 5, `ws` 8.22, webpack-cli 7, `@devicefarmer/adbkit` 3.3.9 al posto del fork `@dead50f7/adbkit` 2.11 (cinque punti di chiamata, tutti dietro `src/server/goog-device/adb.ts`), `tsconfig` moderno, configurazione di webpack in JavaScript (`webpack.config.cjs`). Versioni fissate.
- `src/packages/multiplexer`: `// @ts-nocheck` (imita WebSocket con le vecchie tipizzazioni del DOM; il flusso vero lo prova).

## Provare e costruire

```bash
npm ci --ignore-scripts
npm run typecheck
npm run dist     # dist/index.js e dist/public
npm run smoke    # senza telefono: pagina, rifiuti, impronta del jar
```

L'immagine (`Dockerfile`) fa lo stesso in due fasi e parte come utente 10001, sistema di file in sola lettura.

## Aggiornare da upstream

Prendere i commit di NetrisTV dopo `cd6cea6`, riportare nella cartella solo quello che serve (le parti tolte restano tolte), `npm run typecheck`, `npm run dist`, `npm run smoke`, poi provare su telefono vero (tocco, trascinamento, tasti, accensione) con `OE-Copilot-prove/ws-*.mjs`.

## Server sul telefono: perché resta 1.19-ws8

Provato il 2026-10-11 il server ufficiale scrcpy 5.0.1 (la versione attuale) sullo stesso telefono, con la stessa animazione e lo stesso limite (15 fps, 1 Mbit/s, 720x1280): 1,27 core nel telefono contro 1,27 a 1,34, 12,2 fotogrammi al secondo e 457 kbit/s contro 11,8 a 13,1 e 417 a 504. Il costo sta nel disegno dello schermo senza GPU, non nel codificatore, e il server 5 non parla il protocollo che il browser usa (socket con `scid`, pacchetti di sessione, messaggi di controllo nuovi): servirebbe un ponte da scrivere e provare, senza vantaggio. Il fork 1.19 su Android 13 scrive un'eccezione del `ClipboardManager` per ogni client (solo la copia degli appunti, il flusso e i tocchi non ne risentono).
