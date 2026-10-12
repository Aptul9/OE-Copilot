# OE Copilot, guida completa

Guida per installare OE Copilot: il **server Linux** (la "VM") che legge i calendari e li mostra in un browser. Spiega anche *perché* è fatto così, perché le scelte non sono ovvie e senza il ragionamento è facile rifarle male.

> In breve: OE Copilot unisce in un calendario unico tutte le caselle (aziendali e personali), con un colore per azienda. Le caselle le legge un agente sul server, da un telefono Android virtuale in cui gli accessi si fanno una volta sola. Si vedono **nel browser**: il server serve la pagina dietro HTTPS e una pagina di accesso (sezione 4.7), e sul PC non si installa niente. L'app per Windows, in Python, è stata ritirata l'11/10/2026 (sezione 3).
>
> Sul PC Outlook non serve: gli accessi stanno sul server.

Indice:
1. [I ragionamenti (perché è fatto così)](#1-i-ragionamenti-perché-è-fatto-così)
2. [Architettura](#2-architettura)
3. [L'app per Windows (ritirata)](#3-lapp-per-windows-ritirata)
4. [Installazione sul server](#4-installazione-sul-server)
5. [Prove dal PC: tunnel verso l'agente](#5-prove-dal-pc-tunnel-verso-lagente)
6. [Il rilevatore degli accessi scaduti](#6-il-rilevatore-degli-accessi-scaduti)
7. [Manutenzione e diagnostica](#7-manutenzione-e-diagnostica)
8. [Sicurezza e privacy](#8-sicurezza-e-privacy)

---

## 1. I ragionamenti (perché è fatto così)

Queste scelte sono costate tempo: rifarle "di pancia" porta negli stessi vicoli ciechi.

**Perché non Microsoft Graph / Azure.** La strada "pulita" sarebbe un'app registrata su Azure con
Microsoft Graph. Ma i tenant aziendali **bloccano il consenso utente** alle app di terze parti
("È necessaria l'approvazione dell'amministratore"), e basta un solo tenant bloccato per dover
comunque tenere un'altra strada. Anche il trucco del client ID pubblico di prima parte ("Microsoft
Graph Command Line Tools") funziona sull'account personale ma viene bloccato dai tenant aziendali.
Scartato.

**Perché non fare scraping dal browser / OWA.** Fragile: cambia una classe CSS o un flusso di
login e si rompe. Scartato.

**Perché un telefono Android virtuale con l'app Outlook ufficiale.** L'app **Outlook per Android è
di prima parte Microsoft**: è già autorizzata in tutti i tenant, non chiede il consenso
dell'amministratore (dove TbSync/EAS lo chiederebbe, Outlook no). Facendola girare su un telefono
Android *virtuale* sul server, i login stanno lì dentro e il calendario di sistema di Android
diventa la copia che l'agente legge. Niente Azure, niente scraping, niente consenso per-tenant.

**Perché redroid (e non un emulatore).** [redroid](https://github.com/remote-android/redroid-doc)
è Android in un container Docker che gira **nativo** sul kernel dell'host (niente emulazione, niente
virtualizzazione annidata). Su un host ARM64 un telefono ARM gira a velocità piena. Serve solo il
modulo del kernel `binder_linux`.

**Perché i servizi Google sul telefono per i calendari Google.** Outlook per Android **non esporta**
gli account Google nel calendario di sistema di Android (l'interruttore "Sincronizza calendari" è
disattivato per loro). Un telefono con i servizi Google li sincronizza invece da sé: l'account Google
si aggiunge nelle impostazioni come su un telefono vero e il sincronizzatore di Google copia i
calendari in quello di sistema, dove l'agente li legge come quelli di Outlook. L'immagine redroid
non li ha: [`server/immagine-telefono/`](server/immagine-telefono/Dockerfile) ci aggiunge
[MindTheGapps](https://github.com/MindTheGapps) (release ufficiale, impronta controllata, app con la
firma di Google). Un client CalDAV, [DAVx⁵](https://www.davx5.com/), fu provato e tolto dal
repository: il suo login Google richiede un client OAuth proprio, e quello della build OSE non è
più accettato da Google (`Error 401: invalid_client`).

**Perché un servizio che forza la sincronizzazione.** Due vincoli di Android:
- la sincronizzazione periodica ha un **minimo di 15 minuti**;
- Outlook riceve dai servizi Google le notifiche push (FCM), ma sono quelle della **posta**: una cancellazione o una modifica di un appuntamento non ne ha. Il telefono la applica solo quando Outlook è **davvero in primo piano**, cioè a telefono sbloccato (11/10/2026: sette inviti cancellati alle 09:45 UTC arrivati sul telefono alle 11:42:29, appena sbloccato). Prima dei servizi Google non c'erano nemmeno le push: Outlook fermo nelle impostazioni non scaricava niente (05/10/2026, nessuna riunione nuova in 6 minuti).

Il supervisore (`oe-copilot-supervisore`, prima il servizio systemd `oe-copilot-sincronizza`) gira sul server e a ogni giro (circa 35 secondi: 30 di pausa più il lavoro del giro) forza i lavori di sincronizzazione, porta Outlook sulla schermata principale e tiene sveglio il telefono. Così l'aggiornamento è di ~20-30 s invece di 15 min, finché Outlook è in primo piano.

**Perché il telefono ha un PIN e resta sbloccato.** Un criterio di sicurezza del dispositivo, imposto da una delle aziende le cui caselle sono in Outlook, chiede una password del dispositivo (PIN di almeno 4 cifre, senza ripetizioni né sequenze) e altrimenti Outlook rimuove la casella: togliere il PIN l'11/10/2026 ha fatto comparire in Outlook la schermata "Imposta la password del dispositivo". Lo stesso criterio forza lo spegnimento dello schermo, e quindi il blocco, 15 minuti dopo l'ultimo input (`mMaximumScreenOffTimeoutFromDeviceAdmin`; lo "schermo sempre acceso" di Android viene ignorato). Un telefono bloccato non applica cancellazioni e modifiche, per questo il servizio manda un tocco sulla barra di stato ogni 10 giri (circa 6 minuti): conta come attività dell'utente e rimanda il blocco. Il PIN resta, lo digita l'utente e nessuno script lo scrive; una volta digitato il telefono resta sbloccato, e di questo si tiene conto nella [sezione 8](#8-sicurezza-e-privacy).

**Perché un tunnel SSH (e non un'API esposta su internet).** L'agente ascolta **solo sul localhost
del server** e sulla rete di Docker. Il `gateway` lo raggiunge dalla rete di Docker; dal PC, per le
prove, ci si arriva con un tunnel SSH ([sezione 5](#5-prove-dal-pc-tunnel-verso-lagente)): l'accesso
passa dalla tua chiave SSH, più una chiave dell'API, senza porte aperte e senza certificati da gestire.
Il browser passa invece dall'interfaccia web (4.7), l'unica parte di OE Copilot raggiungibile da
internet: HTTPS e una pagina di accesso, che il gateway chiede prima di mostrare i calendari e prima di
far passare lo schermo del telefono (che non ha password propria).

**Perché il rilevatore degli accessi scaduti.** Se un login sul telefono scade (password cambiata,
sessione revocata, MFA), il calendario Android resta com'era e la lettura **riesce lo stesso**: il
pallino resterebbe verde su dati fermi (un "falso verde"). Il rilevatore se ne accorge e mette il
pallino rosso. Vedi la [sezione 6](#6-il-rilevatore-degli-accessi-scaduti).

---

## 2. Architettura

```
 Browser ──HTTPS──► caddy-ui ──► gateway ──► agent ──► calendar.db del telefono (sola lettura)
                        │        (accesso)                  ▲
                        │                                   │
                        └─ /telefono/ ─► schermo ──adb──► telefono (redroid) ◄── oe-copilot-supervisore
                                                                                   (ogni 35 s)
```

- **telefono** = container redroid con l'app Outlook (caselle Microsoft) e i servizi Google (calendari Google). Le app copiano gli appuntamenti nel calendario di sistema di Android.
- **agent** = piccolo server HTTP che legge quel calendario e lo espone con un'API a chiave, in ascolto solo su `127.0.0.1:3180` del server. Espande anche le ricorrenze dei calendari ICS (link) che il gateway gli manda (`POST /api/ics`).
- **supervisore** (`oe-copilot-supervisore`) = tiene aggiornato il telefono, rileva gli accessi scaduti e scrive i "segni di vita" (`battito.json`, `accessi.json`).
- **interfaccia nel browser** (4.7) = `caddy-ui` (HTTPS) e `gateway` (pagina di accesso, pagina in TypeScript, loghi e tutta la logica dell'app: eventi, calendari, configurazione, disponibilità, interruttori, gruppi, alias, link ICS, eventi personali; lo stato sta nel suo database). Il gateway chiede gli eventi all'agent sulla rete di Docker. Il vecchio servizio Python `ui` non c'è più: l'ultimo commit che lo contiene è `142e789`.
- **schermo** (4.8) = il telefono mostrato e comandato nel browser, dietro lo stesso accesso.

Il codice del server sta nella cartella **`server/`** del repository.

---

## 3. L'app per Windows (ritirata)

L'app per Windows (Python, pywebview e Outlook classico) è stata ritirata l'11/10/2026: leggeva le caselle dal server con un tunnel SSH e ci aggiungeva i promemoria di Windows e Telegram, l'avvio con Windows e il pulsante *Apri in Outlook*. L'interfaccia nel browser (4.7) fa lo stesso lavoro di lettura senza installare niente sul PC; il codice dell'app per Windows resta nella storia di git (ultimo commit con l'app: `360c5c7`).

Cosa è cambiato:
- **Promemoria**: non ci sono. Quelli dell'app partivano solo finché l'app era aperta; li riprende un servizio sul server, se e quando lo si decide.
- **Apri in Outlook**: non c'è. Nel browser *Apri in Teams* e *Partecipa* aprono il link della riunione.
- **Estensioni personali** (`personale/`): il meccanismo non esiste più. Una scheda Timesheet con `azione` e senza `url` non compare; con `url` apre il gestionale.
- **Tunnel e chiave dell'agente sul PC** (`agente_remoto` in `personalizzazione.json`, `agente_chiave.bin`): non servono più. Il file locale, se c'è ancora, si può cancellare.

Il successore è il client desktop in `app/` (Tauri, per Windows e, ancora da provare, per Android): una finestra sull'interfaccia nel browser, con icona nell'area di notifica, avvio con Windows e notifiche di Windows per le riunioni in arrivo. Come si costruisce, cosa include e cosa no (*Apri in Outlook* resta escluso) sta in `app/README.md`.

---

## 4. Installazione sul server

### 4.1 Requisiti dell'host
- Un host **Linux** con **Docker** (e il plugin `compose`).
- Kernel con il modulo **`binder_linux`** (serve a redroid). Verifica:
  ```bash
  grep -E 'CONFIG_ANDROID_BINDER_IPC|CONFIG_ANDROID_BINDERFS' "/boot/config-$(uname -r)"
  # devono essere =y o =m
  ```
  Provato su **Ubuntu 24.04 ARM64** (i kernel `-oracle` hanno `binder_linux` come modulo). Va anche
  su x86‑64 purché il kernel abbia il modulo. Su alcuni kernel "cloud" il modulo manca: in quel caso
  serve un kernel che lo includa (es. `linux-image-generic`/`-oracle`) o `linux-modules-extra`.
- Risorse: il telefono usa ~1,5 GB di RAM e ~2 GB di disco; conta qualche GB in più per immagini
  Docker e app. Una VM da 2–4 GB di RAM basta.

Carica il modulo binder all'avvio, con le opzioni giuste:
```bash
echo binder_linux | sudo tee /etc/modules-load.d/oe-copilot-telefono.conf
echo 'options binder_linux devices=binder,hwbinder,vndbinder' | sudo tee /etc/modprobe.d/oe-copilot-telefono.conf
sudo modprobe binder_linux devices=binder,hwbinder,vndbinder
lsmod | grep binder_linux   # deve comparire
```

### 4.2 Prendere il codice
```bash
git clone https://github.com/<proprietario>/OE-Copilot.git
sudo cp -a OE-Copilot/server /opt/oe-copilot
cd /opt/oe-copilot
```

Crea i due file di configurazione (restano solo sul server, non nel repository):
```bash
# chiave dell'API: lunga e casuale
printf 'API_KEY=%s\n' "$(openssl rand -hex 32)" | sudo tee .env >/dev/null
sudo chmod 600 .env

# elenco delle caselle da leggere: copia l'esempio e mettici le tue
sudo cp agent/fonti.esempio.json agent/fonti.json
sudoedit agent/fonti.json
```
`fonti.json` è una lista di caselle, una riga per casella:
```json
[
  {"tipo": "android", "id": "android::mario.rossi@azienda.example", "nome": "Azienda",
   "account": "mario.rossi@azienda.example"}
]
```
`account` è l'indirizzo esatto come lo vedi in Outlook sul telefono; `nome` è l'etichetta che
vuoi mostrare; `id` è un identificativo qualsiasi ma univoco.

Di una casella Outlook l'agente legge solo il calendario principale. Per leggerne altri (i
calendari che altre caselle o altre aziende condividono con quella, "People's calendars") elencali
per nome nella voce, **principale compreso**:
```json
{"tipo": "android", "id": "android::mario.rossi@azienda.example", "nome": "Azienda",
 "account": "mario.rossi@azienda.example", "calendari": ["Calendar", "Beta", "Gamma"]}
```
Il nome è quello che si vede in Outlook. Ogni evento porta con sé il nome del suo calendario, e
`azienda_per_cartella` (in `personalizzazione.json`) lo usa per mostrarlo come l'azienda giusta, per esempio
`"azienda_per_cartella": {"· Beta": "Beta"}`. Per dare a quell'azienda una voce, un colore e un
pallino propri serve anche una voce in `aziende_inoltrate` con `via` uguale alla casella che la
ospita.

### 4.3 Avviare lo stack
```bash
sudo docker compose up -d telefono agent    # avvia 'telefono' (redroid) e 'agent'
sudo docker ps                   # devono essere "Up" entrambi
```
Il primo avvio scarica le immagini. Se il telefono non parte, quasi sempre manca il modulo binder
(vedi 4.1).

### 4.4 Configurare il telefono virtuale
**Impostazioni e app.** Fuso orario, lingua, schermo sempre acceso, l'app Outlook, niente
risparmio batteria per Outlook e il permesso sul calendario del sincronizzatore di Google li mette
lo script, senza toccare lo schermo:
```bash
sudo /opt/oe-copilot/strumenti/prepara_telefono.sh
# altro fuso o altra lingua: sudo FUSO=Europe/London LINGUA=en-GB .../prepara_telefono.sh
```
Si può rilanciare quanto si vuole (fa solo quello che manca) e l'aggiornamento automatico della
[sezione 4.6](#46-aggiornamento-automatico-facoltativo) lo lancia a ogni giro: un telefono ricreato
da zero torna a posto da solo, restano da rifare solo i login.

L'app Outlook esiste solo sul Play Store, quindi l'APK
(**arm64‑v8a**: redroid qui è 64‑bit, gli APK armeabi‑v7a falliscono) sta fra i file della release
[`apk`](../../releases/tag/apk) del repository. Pesa più dei 100 MB che GitHub accetta per un file
nel repository, per questo è lì e non fra i sorgenti. L'aggiornamento automatico lo scarica e lo
porta in `/opt/oe-copilot/apk/`; a mano:
```bash
gh release download apk --repo <proprietario>/OE-Copilot --pattern 'Outlook-*.apk' --dir /opt/oe-copilot/apk
```
Lo script installa solo l'APK elencato in `apk/SHA256SUMS` (che è nel repository) e solo se
l'impronta coincide. Che sia davvero l'app di Microsoft lo dice la firma, che Android controlla
all'installazione: il certificato ha SHA‑1 (base64) `fcg80qvoM1YMKJZibjBwQcDfOno=` (è pubblico, lo
stesso del redirect `msauth://com.microsoft.office.outlook/…`).

Per passare a una versione nuova di Outlook: carica l'APK nella release
(`gh release upload apk <file>`), sostituisci la riga in `server/apk/SHA256SUMS` con
`sha256sum <file>` e fai push. Su un telefono che ha già Outlook lo script non lo reinstalla:
lì l'app si aggiorna con `docker cp` e `pm install -r`, i login restano.

> Aurora Store in sessione anonima, da un server in cloud, non funziona: il suo servizio di accesso
> risponde 403 agli indirizzi dei datacenter (provato da Oracle Cloud il 09/10/2026).

**Schermo.** Il telefono si vede e si comanda dal browser, dalla pagina `/telefono/` dell'interfaccia web ([sezione 4.8](#48-lo-schermo-del-telefono-nel-browser-facoltativo)): è il modo per fare i login, digitare il PIN dopo un riavvio e inserire i codici MFA. Senza quel servizio il telefono si vede dal **tuo PC** con [scrcpy](https://github.com/Genymobile/scrcpy) (`scoop install scrcpy`, oppure lo zip dal sito: contiene anche `adb`), attraverso un tunnel SSH verso la porta adb del telefono (la `5555`, esposta solo sul localhost del server). Il file `PAUSA` ferma il servizio di sincronizzazione, che altrimenti porterebbe Outlook davanti a ogni giro e ti "ruberebbe" lo schermo:

```bat
ssh <utente@server> "sudo touch /opt/oe-copilot/telefono/PAUSA"
ssh -N -L 15555:127.0.0.1:5555 <utente@server>
rem in un altro prompt:
adb connect 127.0.0.1:15555
scrcpy -s 127.0.0.1:15555
rem alla fine:
ssh <utente@server> "sudo rm /opt/oe-copilot/telefono/PAUSA"
```

**Login**, sullo schermo del telefono:
1. **Caselle Microsoft** in Outlook. Per **ogni** casella, dopo averla aggiunta, apri
   *Impostazioni → Calendario → Sincronizza calendari* e **accendi l'interruttore**: è **per
   account**, e le caselle aggiunte dopo partono **spente** (è il motivo tipico per cui un
   calendario "non arriva").
   Dei calendari **secondari e condivisi** Outlook passa al telefono solo quelli **spuntati**
   nell'elenco dei calendari (vista Calendario → menu in alto a sinistra → la casella): senza
   spunta il calendario esiste sul telefono ma resta vuoto. Spunta quelli che vuoi leggere, poi
   elencali in `fonti.json` ([sezione 4.2](#42-prendere-il-codice)).
2. **Calendari Google.** Serve quando l'indirizzo segreto iCal non c'è (l'amministratore del
   dominio può toglierlo: in *Integra calendario* resta solo l'indirizzo pubblico); se c'è, il link
   nell'app è più semplice e il telefono non serve. Aggiungi l'account Google come su un telefono
   vero: *Impostazioni → Password e account → Aggiungi account → Google* (la schermata si apre
   anche con `adb shell am start -a android.settings.ADD_ACCOUNT_SETTINGS --esa account_types com.google`).
   Il sincronizzatore di Google copia nel calendario del telefono solo i calendari **spuntati**
   (app Calendario del telefono → *Calendari da sincronizzare*): quelli condivisi da altri account
   esistono sul telefono ma restano a 0 eventi finché non li spunti. Se Google dice che il
   dispositivo non è certificato, l'ID del dispositivo (GSF) si registra su
   https://www.google.com/android/uncertified/; con l'account provato non è servito.
3. Scrivi le caselle in `agent/fonti.json` ([sezione 4.2](#42-prendere-il-codice)) e riavvia
   l'agente: `sudo docker compose restart agent`. Se l'agente in TypeScript è acceso (opzione, [sezione 5](#5-prove-dal-pc-tunnel-verso-lagente)) legge lo stesso file: si riavvia anche lui, `sudo docker compose restart agente-ts`.

> I login (password e MFA) li fai **tu**, dallo schermo del telefono.

### 4.5 Il servizio di sincronizzazione
Il servizio di sincronizzazione è il supervisore: tiene aggiornato il telefono e scrive i segni di vita. È il servizio `supervisore` di compose (`supervisore/`, Node.js e TypeScript, adb sulla rete di compose). Lo ricrea l'aggiornamento automatico (4.6) a ogni deploy; a mano si avvia con `docker compose --profile supervisore up -d --build --no-deps supervisore`. Il vecchio servizio systemd `oe-copilot-sincronizza` (lo script `strumenti/sincronizza_telefono.sh`, con il rilevatore in Python) resta nel repository come ritorno indietro, spento: non va mai acceso insieme al supervisore.
Un giro dura circa 35 secondi (30 di pausa, `OGNI`, più il lavoro del giro) e fa questo:
- forza i lavori di sincronizzazione del calendario di Outlook per ogni casella e, ogni 4 giri (`GOOGLE_OGNI`), quelli di Google;
- se lo schermo è spento e nessuna pagina del telefono è aperta, lo riaccende; poi Outlook esce dallo schermo (si porta davanti la schermata iniziale di Android) e subito dopo **torna sulla schermata principale**. Con quel ritorno Outlook ha scaricato le cancellazioni (11/10/2026, 11:42:26 UTC); che serva anche restando sempre davanti non è provato;
- scrive `stato/battito.json` (`quando`, `pausa`, `telefono`, `outlook_davanti`, `bloccato`);
- ogni 2 giri (`ACCESSI_OGNI`, circa 70 secondi) lancia il **rilevatore degli accessi** (sezione 6);
- ogni 10 giri (`TOCCO_OGNI`, circa 6 minuti, `0` spegne) manda un tocco sulla barra di stato a schermo acceso.

Con il file `PAUSA` (`telefono/PAUSA`, oppure `stato/schermo/PAUSA` finché una pagina `/telefono/` è aperta, 4.8) il servizio non porta Outlook in primo piano e forza solo le sincronizzazioni di Android.

**Telefono sbloccato.** Outlook applica una cancellazione o una modifica solo quando è davvero davanti, cioè a telefono sbloccato; a telefono bloccato arrivano solo le push della posta. Il criterio del dispositivo di Outlook (sezione 1) blocca il telefono 15 minuti dopo l'ultimo input, e il tocco sulla barra di stato rimanda il blocco: dopo il PIN il telefono resta sbloccato, e chi arriva alla pagina del telefono (dopo l'accesso) lo trova sbloccato. Per tornare al blocco dopo 15 minuti si imposta `TOCCO_OGNI=0` nel servizio `supervisore` di `docker-compose.yml`, nel repository, e si ridistribuisce.

**Il PIN non si può togliere.** Il criterio chiede una password del dispositivo: toglierla (`cmd lock_settings clear --old <PIN>` dal server) l'11/10/2026 ha fatto comparire in Outlook la schermata "Imposta la password del dispositivo", con l'avviso che altrimenti la casella viene rimossa. Il PIN si rimette con `cmd lock_settings set-pin <PIN>` dal server (le schermate del PIN in Impostazioni escono nere nella pagina). Lo sblocco lo digita sempre l'utente: se lo schermo è spento il servizio lo riaccende, sulla schermata di blocco, e non manda altro.

**Il supervisore: utente, cartelle e ritorno indietro.** `supervisore/` (Node.js, TypeScript) fa il lavoro dello script e del rilevatore: stessi giri, stesse variabili d'ambiente (`OGNI`, `ACCESSI_OGNI`, `GOOGLE_OGNI`, `TOCCO_OGNI`, `PAUSA_SCHERMO_SCADE`, le soglie della sezione 6) e stessi file in `stato/`. Parla col telefono con adb sulla rete di compose (`telefono:5555`, come il servizio schermo) invece di `docker exec`, quindi non ha `docker.sock` e non gira come root.È il servizio `supervisore` di compose con `profiles: ["supervisore"]` (un `docker compose up` semplice non lo avvia: lo avvia l'aggiornamento automatico, 4.6), e non va mai avviato insieme al servizio systemd (due giri sullo stesso telefono si pesterebbero). Gira come l'utente 10080, quello di Outlook sul telefono (`u0_a80`): database e log di Outlook sono leggibili solo da lui e da root (un container senza capacità, root compreso, non li legge) e adb non li raggiunge (parla come l'utente `shell`), quindi restano file del server, montati in sola lettura. `stato/` deve essere suo, `stato/schermo` resta dell'utente 10001:
```bash
cd /opt/oe-copilot
sudo chown 10080:10080 stato
sudo install -d -o 10001 -g 10001 -m 755 stato/schermo       # solo se manca (di solito c'è già)
sudo systemctl disable --now oe-copilot-sincronizza
docker compose --profile supervisore up -d --build --no-deps supervisore
docker inspect -f '{{.State.Health.Status}}' oe-copilot-supervisore       # healthy entro circa un minuto
cat stato/battito.json; stat -c '%y' stato/accessi.json                    # battito ogni 35 secondi, accessi ogni 70
```
Per tornare al servizio systemd: `docker compose --profile supervisore rm -sf supervisore`, poi `sudo ln -sf /opt/oe-copilot/strumenti/oe-copilot-sincronizza.service /etc/systemd/system/`, `sudo systemctl daemon-reload` e `sudo systemctl enable --now oe-copilot-sincronizza` (`disable` toglie il collegamento della unit, senza il `ln` la unit non si trova) e il ritorno del commit del passaggio (`git revert`), perché `aggiorna_server.sh` di adesso rifiuta di andare avanti con il servizio systemd acceso. L'aggiornamento automatico (4.6) ricrea il supervisore a ogni deploy e controlla con `docker inspect` che giri. Il tocco sulla barra di stato parte da adb come utente `shell` e non da root, e conta come attività dell'utente: lo si è visto il 12/10/2026 al passaggio, l'età dell'ultima attività (`dumpsys power`, `mLastUserActivityTime`) si azzera a ogni tocco (giro 0, giro 10) e il telefono è rimasto sbloccato oltre i 15 minuti dall'ultimo tocco del vecchio servizio. Per ricontrollarlo: `dumpsys power` prima e dopo un giro con il tocco.

Verifica che l'agente risponda (dal server):
```bash
curl -s -H "Authorization: Bearer $(sudo grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2)" \
  http://127.0.0.1:3180/api/calendari | python3 -m json.tool | head
```

### 4.6 Aggiornamento automatico (facoltativo)
La GitHub Action `.github/workflows/server.yml` porta `server/` sul server a ogni push su `main` che tocca `server/` o il workflow stesso (l'elenco è in `paths` del workflow): controlla la sintassi e le prove (compresi il confronto dell'agente con le risposte registrate in `server/strumenti/golden/` e le prove e la build del gateway, sezione 7), copia il codice con rsync (`--delete`) e lancia `strumenti/aggiorna_server.sh`, che rimette a posto impostazioni e app del telefono (`prepara_telefono.sh`, con l'APK di Outlook preso dalla release `apk`), ricostruisce e ricrea l'agente e il supervisore (senza toccare il telefono) e verifica che l'agente risponda, che il supervisore giri e che il battito sia fresco. Se la verifica fallisce rimette la versione di prima. `.env`, `agent/fonti.json`, `telefono/`, `stato/`, `ui-dati/` e `.versione` non vengono toccati dalla copia (`.versione` lo scrive lo script, con lo SHA distribuito). Il telefono si ricrea solo se cambia la sua parte di `docker-compose.yml`. Se in `.env` c'è `UI_HOST`, a fine aggiornamento partono anche l'interfaccia nel browser (4.7) e lo schermo del telefono (4.8): un loro guasto rende rossa la Action ma non riporta indietro l'agente né il telefono. Dopo aver avviato il gateway nuovo l'aggiornamento toglie, se c'è ancora, il container `oe-copilot-ui` del servizio Python che non esiste più (e la sua immagine). Un push che tocca solo file fuori da `server/` e dal workflow (per esempio `GUIDA.md` o `README.md`) non lancia niente, e `.versione` resta indietro.

Sul server serve, una volta sola, un utente dedicato che entra solo con una chiave SSH:
```bash
sudo useradd -m -s /bin/bash -G docker oecopilot
# il codice è dell'utente della Action; telefono/ resta di root (lo crea Docker), stato/ è dell'utente 10080 (quello del supervisore, 4.5) con stato/schermo del 10001, ui-dati/ dell'utente 10001
sudo chown oecopilot:oecopilot /opt/oe-copilot /opt/oe-copilot/.env
cd /opt/oe-copilot && sudo chown -R oecopilot:oecopilot agent apk caddy-ui immagine-telefono schermo-web strumenti web-ts docker-compose.yml .env.esempio
```
Con il supervisore l'utente della Action non ha bisogno di `sudo`: prima aveva due comandi `systemctl` in `/etc/sudoers.d/oe-copilot` (`daemon-reload` e il riavvio del servizio di sincronizzazione), su questo server il file c'è ancora e non serve più.

La chiave pubblica della Action va in `/home/oecopilot/.ssh/authorized_keys`. Nel repository
servono quattro secret: `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY` (la chiave privata) e
`DEPLOY_KNOWN_HOSTS` (la riga di `ssh-keyscan -t ed25519 <server>`); con l'interfaccia nel browser (4.7)
anche `ADMIN_EMAIL` e `ADMIN_PASSWORD`. Senza i secret
l'aggiornamento fallisce al collegamento SSH: chi non ha un server disattiva la Action.

### 4.7 L'interfaccia nel browser (facoltativa)
Il calendario unico (caselle, colori, pallini di stato e filtri) con una pagina in TypeScript,
servita dal server a un browser qualunque, con HTTPS e una pagina di accesso. Gli interruttori
(calendari, aziende, alias, calendari da link ICS, eventi personali) restano sul server. Non ci
sono i promemoria (li dava l'app per Windows, sezione 3) né *Apri in Outlook*; i link delle riunioni
si aprono nel browser.

Come è fatta, in `server/docker-compose.yml`:
- **`gateway`** (`server/web-ts/`, Next.js e Better Auth): la pagina di accesso, la pagina in
  TypeScript, i metodi dell'API e i loghi. Senza sessione una pagina rimanda a `/login?next=...` e
  l'API (`/api/*`) risponde 401 in JSON; con la sessione il gateway risponde da sé: i metodi
  (`POST /api/<metodo>`, `server/web-ts/src/lib/core/`: chiede all'agente eventi e calendari e tiene
  lo stato in `ui-dati/gateway.db`) e i loghi (`GET` e `HEAD /logos/<file>`, più sotto); qualunque
  altro percorso che non sia una pagina è 404 in JSON. Il servizio Python `ui`, che prima serviva i loghi, non c'è più
  (l'ultimo commit che lo contiene è `142e789`). Del testo dei link ICS (copia in `ui-dati/ics_cache/`, rinnovata ogni 30 minuti) si
  occupa il gateway; le ricorrenze le espande l'agente (`POST /api/ics`, `server/agent/ics.py`, la stessa
  libreria che legge il telefono) e il gateway tiene in memoria l'ultima espansione riuscita di ogni link e
  intervallo: se l'agente non risponde e il gateway è appena ripartito, un link ICS mostra l'errore. Il
  compose dà al gateway `OE_DATI=/dati` (la cartella dei dati), `GATEWAY_DB=/dati/gateway.db`,
  `AGENTE_URL=http://agent:8180` e `AGENTE_CHIAVE` (la `API_KEY` di `.env`). **Lo stato** (calendari
  spenti, gruppi, alias, colori, link ICS, tema e il resto della configurazione, e gli eventi personali)
  sta nel database, tabelle `stato_config` e `eventi_locali`: ogni modifica è una transazione. La prima
  volta il gateway importa `config.json` e `local_events.json` di `ui-dati/` (se ci sono); da lì in poi il
  database è la fonte e i due file non si rileggono, ma si riscrivono dopo ogni modifica nel formato di
  sempre, così si può tornare a una versione che li leggeva (fino al commit `142e789` c'era il
  servizio `ui`) senza perdere niente: cambiarli a mano non ha effetto, salvo la reimportazione della
  sezione 7. Il cookie è HttpOnly, Secure e SameSite=Lax e dura 30 giorni; dopo 5 password sbagliate
  l'account si blocca 15 minuti, qualunque sia l'indirizzo (dietro sslh e docker-proxy tutti i
  client hanno lo stesso, quindi un limite per indirizzo sarebbe un contatore solo). Valgono le
  regole contro le richieste partite da un altro sito: JSON obbligatorio per i POST e
  `Sec-Fetch-Site` dello stesso sito. Il tasto *Esci* della barra a sinistra chiude la
  sessione. Il proprietario nasce a ogni avvio da `ADMIN_EMAIL` e `ADMIN_PASSWORD` di `.env`; le
  sessioni e il blocco stanno in `ui-dati/gateway.db`. Porta solo su `127.0.0.1:3190`.
- **Interfaccia nel gateway**: la pagina (`/`) è scritta in TypeScript dentro il gateway (`server/web-ts/src/components/oe/`, React e FullCalendar da npm, componenti shadcn) e chiama `POST /api/<metodo>` (i metodi di `server/web-ts/src/lib/core/api.ts`, che il gateway serve tutti), con il layout per telefono e tablet.
- **`caddy-ui`**: Caddy con HTTPS, solo su `127.0.0.1:4445`, che passa tutto al gateway.
  L'indirizzo arriva da `.env`, non dal repository.
- **`ui-dati/`**: `gateway.db` (sessioni, blocco degli accessi e stato dell'app), le copie
  `config.json` e `local_events.json`, le cache, i loghi e `personalizzazione.json` (nomi, sigle e
  loghi delle aziende, modello in `personalizzazione.esempio.json`). Sta solo sul server.

Messa in funzione, una volta sola, sul server:
```bash
# dati dell'interfaccia: i container li scrivono come utente 10001, senza privilegi
sudo install -d -o 10001 -g 10001 -m 700 /opt/oe-copilot/ui-dati
sudo install -o 10001 -g 10001 -m 600 personalizzazione.json /opt/oe-copilot/ui-dati/
# segreto dei cookie di sessione
openssl rand -hex 32
```
In `.env` (modello in `.env.esempio`): `UI_HOST=<nome del sito>` e `BETTER_AUTH_SECRET=<il segreto>`. Il
prossimo aggiornamento costruisce e avvia `gateway` e `caddy-ui`.

**Email e password del proprietario** (`ADMIN_EMAIL`, `ADMIN_PASSWORD`, almeno 12 caratteri, senza `$`) stanno
nei secret della Action e non in `.env`: l'aggiornamento (4.6) li passa allo script sullo standard input di SSH,
e compose li legge dall'ambiente prima che da `.env`. Per cambiarli:
`gh secret set ADMIN_PASSWORD` e un nuovo aggiornamento (anche `Run workflow` dalla pagina della Action):
il gateway riparte, la password nuova chiude le sessioni aperte. Con una email nuova nasce un utente nuovo e
quello vecchio resta con la sua password: va tolto da `gateway.db` (tabelle `session`, `account`, `user`).
Se i secret mancano vale ancora `.env`, e una `docker compose up` a mano senza i secret ricrea il gateway
senza proprietario nuovo (gli utenti già nel database restano).

**HTTPS.** Caddy ottiene da solo il certificato, e per farlo il suo nome deve arrivargli da
internet. Dove la 443 è libera basta pubblicare `caddy-ui` su `443:443` e `80:80` al posto di
`127.0.0.1:4445:443`. Sulla VM della guida la 443 è di sslh (SSH e HTTPS sulla stessa porta): si
aggiunge a `/etc/sslh.cfg`, **sopra** la voce che prende tutto il resto, una voce per nome
```
{ name: "tls"; host: "127.0.0.1"; port: "4445"; sni_hostnames: [ "oe.<IP con i trattini>.sslip.io" ]; },
```
e si riavvia sslh (`KillMode=process`: le connessioni aperte non cadono). sslip.io risolve da
solo il nome nell'indirizzo, non serve DNS. La porta 80 è di un altro Caddy, quindi il certificato
arriva col TLS-ALPN, che passa da sslh: il primo tentativo HTTP-01 fallisce ed è previsto.

Prova: senza sessione `curl -I https://<UI_HOST>/` risponde 307 verso `/login?next=%2F` e
`curl -X POST -H 'Content-Type: application/json' -d '[]' https://<UI_HOST>/api/get_config`
risponde 401; con il cookie di `POST /api/auth/sign-in/email` (`curl -c jar`, poi `-b jar`) la
pagina risponde 200.

**Secondo fattore dell'accesso (facoltativo).** La variabile `AUTH_2FA` di `.env` (modello in `.env.esempio`), letta a ogni avvio del gateway, decide cosa chiede l'accesso dopo la password:
- `off` (predefinito, vale anche se la variabile manca o è vuota): non cambia niente. Email e password come sempre; le strade del secondo fattore (`/api/auth/two-factor/*`, `/api/auth/passkey/*`) e la pagina `/sicurezza` rispondono 404.
- `totp`: chi ha attivato il codice, dopo la password deve scrivere il codice di 6 cifre di un'app di autenticazione (Google Authenticator, Microsoft Authenticator, Aegis, 1Password e simili) oppure un codice di recupero. Chi non l'ha attivato entra come prima.
- `passkey`: tutto quello di `totp`, più le passkey. Nella pagina di accesso compare *Accedi con una passkey* (basta il dito, il volto o il PIN del dispositivo, niente password); la password resta come alternativa e, per chi ha il codice attivo, passa sempre dal codice. Una passkey è già un accesso completo e non chiede il codice. Il nome del sito e l'origine della passkey vengono da `APP_URL` (`https://<UI_HOST>`): se l'indirizzo del sito cambia, le passkey vecchie non valgono più e ne serve una nuova (nel frattempo basta la password).
- Un altro valore (un refuso) ferma l'avvio del gateway con `AUTH_2FA non valido` nel log (`docker logs oe-copilot-gateway`).
Sul server in uso c'è `AUTH_2FA=totp` dal 12/10/2026: finché il proprietario non attiva il TOTP dalla pagina `/sicurezza` (punto 1 qui sotto) l'accesso resta con la sola password.

Le tabelle del secondo fattore (`twoFactor`, `passkey` e la colonna `user.twoFactorEnabled` in `ui-dati/gateway.db`) il gateway le aggiunge a ogni avvio, anche con `off`: si aggiunge e non si toglie niente, e l'immagine di prima continua a funzionare sullo stesso database (non conosce il secondo fattore e lascia entrare con la sola password). Il segreto del codice sta cifrato con `BETTER_AUTH_SECRET`: cambiare quel segreto rende inutilizzabili i codici già attivati (punto 3 del recupero).

**Attivare il codice** (lo fa il proprietario, 3 minuti):
1. Sul server, in `/opt/oe-copilot/.env`, `AUTH_2FA=totp` (o `passkey`), poi `cd /opt/oe-copilot && sudo docker compose up -d gateway`. Un `docker restart` non basta: l'ambiente di un container si fissa quando lo si crea. Il deploy non tocca `.env`.
2. Aprire il sito, entrare con la password, *Impostazioni*, *Sicurezza*, *Gestisci l'accesso* (o direttamente `https://<UI_HOST>/sicurezza`).
3. *Attiva*, scrivere la password, inquadrare il QR con l'app di autenticazione (o scrivere nell'app la chiave sotto il QR), scrivere il codice che l'app mostra e *Conferma e attiva*. Fino alla conferma l'accesso resta quello di prima.
4. Salvare subito i 10 codici di recupero che compaiono: non si rivedono, ognuno vale una volta sola al posto del codice dell'app. *Nuovi codici di recupero* ne fa altri 10 e annulla i vecchi; *Disattiva* toglie il codice; tutti e due chiedono la password.
5. Con `passkey`: nella stessa pagina *Aggiungi una passkey*, un nome, e il dispositivo chiede dito, volto o PIN. Serve HTTPS (l'indirizzo del sito lo è). Registrare una passkey chiede una sessione aperta da meno di un giorno: con una più vecchia la pagina dice di uscire e rientrare.
6. Provare in una finestra anonima: password, poi codice.

**Blocco dei tentativi.** I codici sbagliati non finiscono nella tabella `login_failures` delle password: li conta il plugin sulla riga `twoFactor` dell'account (`failedVerificationCount`, `lockedUntil`), con gli stessi numeri. Cinque errori di fila, tra codice dell'app e codici di recupero, bloccano i codici per 15 minuti anche per il codice giusto; un codice giusto azzera il conto. Il limite per indirizzo (dietro sslh e docker-proxy tutti i client hanno lo stesso) è di 30 richieste al minuto sui codici, come per la password. Il codice dell'app vale per tre finestre di 30 secondi e non è a uso singolo: il plugin non ricorda quelli già usati. La password sbagliata conta come prima in `login_failures`, anche per chi ha il codice; una password giusta non conta mai come errore.

**Cancello di Caddy.** `/telefono/` chiede al gateway `/api/verifica-sessione` come prima. La sessione nasce solo quando anche il codice è passato: finché manca, il gateway non dà nessun cookie di sessione e il cancello risponde 401 (302 verso l'accesso per una pagina), quindi anche il telefono resta chiuso.

**Recupero** (il proprietario non entra più). In ordine:
1. Il telefono con l'app non c'è ma i codici di recupero sì: nella pagina di accesso, dopo la password, *Usa un codice di recupero*.
2. Niente codici, o il gateway non fa entrare per un altro motivo: `AUTH_2FA=off` e gateway ricreato. Con `off` basta la password, anche per un account col codice attivo e anche se i codici l'hanno bloccato:
   ```bash
   cd /opt/oe-copilot
   sudo sed -i 's/^AUTH_2FA=.*/AUTH_2FA=off/' .env
   sudo docker compose up -d gateway
   docker logs --tail 5 oe-copilot-gateway    # deve dire: secondo fattore dell'accesso (AUTH_2FA): off
   ```
   Poi si entra con la password. Tornando a `totp` il codice attivato vale ancora: non si perde niente cambiando modalità.
3. Per ripartire da zero col codice (telefono e codici persi, o `BETTER_AUTH_SECRET` cambiato) si toglie l'attivazione dal database, con il gateway fermo, e la si rifà dalla pagina *Sicurezza*:
   ```bash
   docker stop oe-copilot-gateway
   docker run --rm --user 10001:10001 -v /opt/oe-copilot/ui-dati:/dati oe-copilot-gateway \
     node -e "new (require('better-sqlite3'))('/dati/gateway.db').exec('DELETE FROM twoFactor; UPDATE user SET twoFactorEnabled = 0')"
   docker start oe-copilot-gateway
   ```
   Le passkey non si toccano. Un aggiornamento del server non cambia `.env` né il database: la modalità scelta resta.
4. Tornare all'immagine di prima (revert della modifica e deploy) funziona sullo stesso database: l'immagine vecchia non conosce il secondo fattore e lascia entrare con la password.

**Gruppi di calendari.** Nella finestra *Calendari da mostrare* (*Gestisci calendari*, a sinistra) si creano, rinominano ed eliminano i gruppi, e i pulsanti accanto al nome di ogni calendario lo mettono dentro o fuori da un gruppo: un calendario può stare in più gruppi. Nella barra a sinistra un pulsante per gruppo lascia accesi solo i suoi calendari (un secondo clic li riaccende tutti). Stanno nella configurazione, voce `gruppi_calendari`, per nome come li mostra la barra: `{"gruppo1": ["Alfa", "Beta"], "gruppo2": ["Gamma"]}`; se si rinomina l'azienda dalla finestra, i gruppi seguono il nuovo nome.

**Disponibilità pubblica.** `https://<UI_HOST>/disponibilita` si apre senza accesso: è lo stesso calendario (viste, minicalendario, zoom, tema) con ogni impegno ridotto a un blocco "Occupato", i soli orari. Impegni che si sovrappongono o si toccano sono un blocco solo; le festività restano. Non ci sono titoli, luoghi, partecipanti, nomi di calendari o di aziende, né ricerca, nuovo evento, legenda, timesheet e impostazioni. `?gruppo=gruppo1` conta solo i calendari di quel gruppo. Contano tutti i calendari, anche quelli spenti nella barra laterale (gli interruttori sono un filtro della vista), e i giorni interi non occupano; se una sorgente non risponde la pagina lo scrive. I dati: `GET /api/disponibilita?da=AAAA-MM-GG&a=AAAA-MM-GG[&gruppo=nome]` (al più 62 giorni), nella forma di `get_events` con solo "Occupato". Il gateway li legge a blocchi di 28 giorni, li tiene un minuto e fa al più 20 letture al minuto in tutto (dietro sslh i client non si distinguono); ogni blocco lo legge il metodo `get_disponibilita` (solo orari, `server/web-ts/src/lib/core/`).

**Loghi delle aziende.** Accanto al nome di ogni azienda nella barra a sinistra, e sugli eventi del calendario (nell'angolo, e nelle righe della vista mese e agenda), compare il suo logo; senza logo restano le iniziali nel colore dell'azienda. Si dichiarano in `ui-dati/personalizzazione.json`, voce `interfaccia.loghi`: il nome come lo mostra la barra e il file, per esempio `{"interfaccia": {"loghi": {"Alfa": "logos/alfa.png"}}}`. I file stanno sul server in `ui-dati/logos/` (PNG quadrati, 128 px bastano; la cartella non è nel repository, come il resto dei dati di chi usa l'app) e il gateway li serve a `GET /logos/<file>` (anche `HEAD`) dopo l'accesso. Se il file non c'è lì, si cerca tra quelli che porta l'immagine del gateway (`server/web-ts/logos-predefiniti/`, in `/app/logos-predefiniti` nel container; la cartella si cambia con `LOGHI_PREDEFINITI`): oggi solo `personale.png`, il logo degli *Eventi personali*, che la pagina chiede da sé senza dichiararlo in `personalizzazione.json`. A parità di nome vince `ui-dati/logos/`: un `personale.png` messo lì sostituisce quello dell'immagine. Il tipo viene dall'estensione (png, jpg, jpeg, gif, webp, svg, ico; qualunque altra è `application/octet-stream`). Un nome che esce dalla cartella (con `..`, assoluto, o un collegamento che porta fuori), una cartella o un file che non c'è risponde 404. Non si riavvia niente: un file nuovo si serve subito, e `personalizzazione.json` il gateway lo rilegge quando cambia. Per aggiungere un'azienda: copiare il file in `ui-dati/logos/` (leggibile dall'utente 10001, come il resto di `ui-dati/`) e aggiungere la riga. La pagina pubblica `/disponibilita` non mostra né nomi né loghi.

**Personalizzazione.** Senza `personalizzazione.json` in `ui-dati/` le aziende prendono il nome dal
dominio della casella e i colori dalla tavolozza. Si parte da `personalizzazione.esempio.json`, copiato
in `ui-dati/` come `personalizzazione.json`. Il gateway rilegge il file quando cambia: dopo una
modifica non si riavvia niente. I colori assegnati stanno nella configurazione (`account_colors`).

### 4.8 Lo schermo del telefono nel browser (facoltativo)
Il servizio **`schermo`** (`server/schermo-web/`, copia potata e aggiornata di ws-scrcpy, provenienza e modifiche in `UPSTREAM.md`) mostra il telefono virtuale in una pagina e lo comanda con mouse e tastiera: serve per il PIN dopo un riavvio, un accesso, un codice MFA, al posto di scrcpy sul PC con un tunnel SSH (4.4).
- Container in sola lettura, utente 10001, senza capacità, `cpus: 1`, 256 MB. Parla con il telefono con adb su `telefono:5555` (rete di compose); un solo telefono, nessuna shell, nessun elenco o caricamento di file, il tunnel porta solo al server scrcpy del telefono (`tcp:8886`, in ascolto solo su localhost nel telefono).
- Non è pubblicato su nessuna porta e non ha password. Il telefono non chiede conferme ad adb (`ro.adb.secure=0`): chi raggiunge il servizio comanda il telefono, quindi l'unico cancello è l'accesso davanti.
- Qualità di partenza: 15 fotogrammi al secondo, 1 Mbit/s, 720x1280. Nel telefono costa quasi niente a schermo fermo e fino a 1,3 core mentre lo schermo si anima di continuo (misure e confronto con il server scrcpy 5 nella decisione 2026-10-11-oe-copilot-phone-browser del vault).
- Il server di scrcpy gira nel telefono (`app_process`, circa 0,02 core fermo) dal primo avvio del servizio e resta lì anche a servizio fermo. Per spegnerlo: tasto *Kill server* nella pagina dei dispositivi, oppure `docker exec oe-copilot-telefono kill <pid>` con il pid della colonna *Server PID*.
- **Come si usa.** Nell'app nel browser, nella barra a sinistra, il pulsante *Telefono* (non c'è nella pagina pubblica `/disponibilita`) apre in una scheda nuova `https://<UI_HOST>/telefono/`, già sul telefono e adattato alla finestra (`#!action=stream&udid=telefono%3A5555&player=webcodecs&fitToScreen=true`; la pagina dei dispositivi sta a `/telefono/` senza il resto). Tocchi e trascinamenti con il mouse; la tastiera del computer è collegata al telefono da subito (il pulsante con la tastiera, l'ultimo della barra a destra, è acceso; un clic lo spegne): lettere, maiuscole, cifre e simboli (`@ . : _ - + /` e gli altri) arrivano come caratteri qualunque sia la tastiera del computer (italiana, americana), le lettere accentate partono come accento morto più lettera (la composizione la fa Android), Ctrl+V incolla il testo del computer, Invio, Backspace, Tab, Esc e le frecce partono come tasti; Ctrl, Alt e F1..F12 restano al browser. Non arrivano i caratteri che la tastiera di Android non ha (€, £, emoji). Il pulsante `?` in cima alla barra a destra apre un promemoria dei comandi (mouse, tastiera, pulsanti, sblocco e accesso), con i tasti in evidenza. Il pulsante `...` in alto apre una casella di testo: quello che si scrive o si incolla lì parte con *Send as keys* (a pezzi di 200 caratteri), utile per una password già copiata; *Set clipboard* e *Get clipboard* non funzionano su Android 13 con questo server. Le cifre della tastiera arrivano al telefono come tasti 0..9 (provato con `dumpsys input`); sul tastierino del PIN non sono mai state provate, perché il PIN lo digita solo l'utente, e il tastierino si può premere anche con il mouse; il pulsante in alto della barra è il tasto di accensione, che accende anche lo schermo quando il telefono, bloccato, lo spegne dopo circa 10 secondi senza tocchi. Il PIN e i codici si scrivono come sempre dall'utente: nessuno script li digita.
- **Accesso.** Caddy (`server/caddy-ui/Caddyfile`) chiede al gateway (`/api/verifica-sessione`, `forward_auth`) se la richiesta ha una sessione valida prima di passare `/telefono/*` al servizio: senza sessione una pagina va all'accesso, il resto riceve 401 e un WebSocket non si apre. Un WebSocket parte solo dalla pagina dello stesso sito (controllo dell'`Origin`, 403 altrimenti). Il percorso `/telefono` senza barra finale rimanda a `/telefono/`.
- **Pausa della sincronizzazione.** Finché una scheda è aperta sul telefono, il servizio riscrive ogni 10 secondi `stato/schermo/PAUSA` (la cartella `stato/schermo`, di utente 10001, si crea una volta a mano, 4.5; `stato/` non si monta intero e `telefono/` mai); il supervisore non porta Outlook in primo piano finché il file è più giovane di 60 secondi e lo ignora dopo, così un servizio morto non blocca niente. Il file si toglie appena si chiude l'ultima scheda; un browser sparito senza chiudere (portatile chiuso, rete caduta) viene chiuso dal servizio dopo 30 secondi senza risposta ai ping. Una scheda lasciata aperta tiene ferma la pausa: si chiude quando si è finito. Il file `/opt/oe-copilot/telefono/PAUSA`, messo a mano (4.4), continua a valere.
- **Spegnerlo.** Togliere il pulsante e la strada: `docker stop oe-copilot-schermo` (da `docker compose` si può anche togliere il servizio `schermo` e ridistribuire; con il servizio fermo `/telefono/` risponde 502 solo a chi ha la sessione). Il server di scrcpy nel telefono resta in esecuzione (circa 0,02 core): per fermarlo, prima di spegnere il servizio, tasto *Kill server* nella pagina dei dispositivi.
- Prova senza telefono: `cd server/schermo-web && npm ci --ignore-scripts && npm run dist && npm run smoke && npm test`.

### 4.9 I promemoria sul server (facoltativo)
Il gateway può avvisare prima delle riunioni su **Telegram** (e, in più, nelle notifiche del browser), anche a pagina chiusa e con il PC spento: è quello che faceva l'app per Windows (sezione 3), ma solo ad app aperta. Di norma è **spento**. Non servono porte aperte: è il server a chiamare i servizi. Il motore è `server/web-ts/src/lib/core/promemoria.ts`, i canali `promemoria-canali.ts` (Telegram) e `promemoria-push.ts` (notifiche del browser, con `public/sw.js`); nessuna dipendenza nuova.

**Attivarlo**, una volta sola:
1. In Telegram scrivi a `@BotFather`, `/newbot`, scegli nome e username: risponde con il **token** (`123456:ABC...`). Il token è una password: sta solo in `.env` sul server e nel lockbox, mai in una chat, in uno screenshot, in un ticket o nel repository.
2. Apri il bot e premi *Start* (un bot non può scrivere per primo). Per un gruppo: aggiungi il bot e scrivi un messaggio nel gruppo.
3. Sul server, in `.env` (`sudoedit /opt/oe-copilot/.env`), aggiungi `TELEGRAM_BOT_TOKEN=<il token>`. L'id della chat si legge con il token preso da `.env`, senza scriverlo sulla riga di comando (finirebbe nella cronologia della shell e nell'elenco dei processi):
   ```bash
   TOKEN=$(sudo grep ^TELEGRAM_BOT_TOKEN= /opt/oe-copilot/.env | cut -d= -f2-)
   printf 'url = "https://api.telegram.org/bot%s/getUpdates"\n' "$TOKEN" | curl -s -K - | grep -o '"chat":{"id":-\?[0-9]*'
   unset TOKEN
   ```
   Esce `"chat":{"id":123456789`: il numero è l'id (negativo per un gruppo). Se non esce niente scrivi un altro messaggio al bot e ripeti.
4. In `.env` aggiungi `TELEGRAM_CHAT_ID=<il numero>` (i nomi sono in `.env.esempio`).
5. Il compose passa i due valori al gateway, che li legge solo all'avvio: `cd /opt/oe-copilot && sudo docker compose up -d gateway`. Un `docker restart` non basta, perché non rilegge `.env`.
6. Nell'app: *Impostazioni*, sezione *Promemoria*. Sotto *Dove* la riga di Telegram non deve più dire "non configurato sul server". *Invia una prova* manda il testo fisso "Prova dei promemoria", mai dati di una riunione: se arriva nella chat il canale funziona. Poi si accende *Avvisami prima delle riunioni*. Senza un canale configurato l'interruttore non si accende.

**Notifiche del browser (il secondo canale, facoltativo).** Il server spedisce il messaggio al servizio di notifiche del browser (Google, Mozilla o Apple, secondo il browser), che lo consegna al browser a pagina chiusa. Il testo viaggia cifrato da un capo all'altro (RFC 8291): il servizio di notifiche vede a chi e quando, non cosa. Non funziona dentro le finestre dell'app Tauri (Windows e Android), che non hanno le notifiche web (misurato il 12/10/2026): per quelle resta Telegram. Serve una coppia di chiavi VAPID, una volta sola, che si fa con node (senza installare niente):
   ```bash
   node -e "const c=require('crypto');const j=c.generateKeyPairSync('ec',{namedCurve:'P-256'}).privateKey.export({format:'jwk'});console.log('VAPID_PUBLIC_KEY='+Buffer.concat([Buffer.from([4]),Buffer.from(j.x,'base64url'),Buffer.from(j.y,'base64url')]).toString('base64url'));console.log('VAPID_PRIVATE_KEY='+j.d)"
   ```
   Le due righe che escono vanno in `.env` (la privata è un segreto come il token: lockbox, mai in chat), con `VAPID_SUBJECT=mailto:<indirizzo>` (o `https://<sito>`), il contatto che il servizio di notifiche può usare se il server si comporta male. Poi `sudo docker compose up -d gateway`. Nelle Impostazioni, sotto *Dove*, *Notifiche del browser* offre *Attiva su questo browser*: il browser chiede il permesso e iscrive sé stesso (si fa una volta per browser o telefono, fino a 20). Poi *Invia una prova* e l'interruttore. *Togli da questo browser* toglie l'iscrizione, e il server toglie da solo quelle che il servizio dichiara scadute. L'indirizzo di un'iscrizione vale come una password: sta solo in `ui-dati/gateway.db` (tabella `push_iscrizioni`), non compare nei log né nelle risposte dell'app, e il server accetta solo indirizzi https con un nome di servizio (mai un indirizzo IP o un nome della rete interna) e non segue i reindirizzamenti. Il messaggio è lo stesso di Telegram: la prima riga è il titolo della notifica, il resto il corpo.

**Cosa arriva.** Testo semplice, senza formattazione. Con *Completo* (di norma):
```
Promemoria: tra 10 minuti, alle 15:30
Riunione di prova
Luogo: Sala 3
Partecipa: https://teams.microsoft.com/l/meetup-join/...
```
I minuti sono quelli che mancano davvero (un avviso in ritardo dice "tra 4 minuti"), l'ora è di Roma, il luogo e il link ci sono se la riunione li ha; per un'altra giornata c'è "domani" o la data. Con *Minimo* restano i minuti, l'ora e l'etichetta dell'azienda, niente titolo, luogo, link o indirizzi.

**Quando parte.**
- Per ogni valore dei *minuti prima* (da uno a cinque valori, da 1 a 1440, di norma 10) un avviso per ogni riunione a orario non ancora iniziata, quando mancano al più quei minuti. I giorni interi non avvisano mai. Vale quello che mostra la pagina: un calendario spento nella barra a sinistra non avvisa. Gli eventi annullati o cancellati non arrivano dal telefono e non avvisano; "libero", "provvisorio" e "rifiutato" l'agente non li legge, e questi eventi avvisano come gli altri. Nei calendari da link (ICS) l'agente non guarda `STATUS` né `TRANSP`: un evento annullato nel calendario di origine compare nella pagina e avvisa.
- Un avviso per riunione, valore dei minuti e canale. Se il server era fermo, l'avviso parte quando torna, se la riunione non è ancora iniziata; se più valori sono già scaduti parte un messaggio solo. Una riunione spostata (inizio nuovo) avvisa di nuovo, una cancellata prima dell'avviso no, le occorrenze di una serie hanno ognuna il suo.
- Il server cerca le riunioni ogni 30 secondi (`PROMEMORIA_INTERVALLO_S`, da 5 a 3600, nel compose): l'avviso arriva fino a 30 secondi dopo il momento esatto. Un evento è fresco quanto il calendario del telefono: una cancellazione fatta in Outlook arriva sul telefono solo a telefono sbloccato (sezione 1), e fino ad allora la riunione cancellata può ancora avvisare.
- Quello che è già partito sta in `ui-dati/gateway.db`, tabella `promemoria_inviati` (chiave, evento, inizio, minuti, canale, ora, esito, tentativi), righe tenute 14 giorni. Un riavvio o un deploy non rimanda niente.
- Se un canale non risponde o rifiuta, l'avviso non si segna come inviato su quel canale e si riprova al giro dopo, fino all'inizio della riunione e al più 5 volte; se il servizio dice di attendere (429) il server aspetta il tempo che indica (al più 15 minuti). Gli altri canali non ne risentono. Nelle Impostazioni si vedono l'ora dell'ultimo controllo e l'ultimo problema (una frase, senza dati delle riunioni). Nel log `docker logs oe-copilot-gateway 2>&1 | grep promemoria`: mai titoli, luoghi, partecipanti, token o chat.

**Privacy.** Con *Completo* titolo, luogo e link passano dai server di Telegram (i messaggi dei bot non sono cifrati da un capo all'altro); con *Minimo* passano solo l'ora e l'etichetta dell'azienda. Nelle notifiche del browser il testo è cifrato da un capo all'altro: il servizio di notifiche non lo legge. Il token e l'id della chat stanno solo in `.env` e nell'ambiente del gateway: non compaiono nei log, nelle risposte dell'app né nelle impostazioni.

**Deploy e riavvio.** Il deploy ricrea il gateway: impostazioni e avvisi già inviati restano in `ui-dati/gateway.db` e il giro riparte 10 secondi dopo l'avvio. Spento (di norma), o senza un canale scelto e configurato, il server non legge eventi per i promemoria e non manda niente. `/sw.js` è un file pubblico (senza accesso), come le icone: contiene solo il codice che mostra la notifica.

**Spegnerlo.** L'interruttore nelle Impostazioni vale dal giro dopo, senza riavvio. Per toglierlo del tutto: via i valori dei canali da `.env` (`TELEGRAM_*`, `VAPID_*`) e `sudo docker compose up -d gateway`.

**Prove.** `cd server/web-ts && npx vitest run test/promemoria`: motore, canale Telegram contro un server HTTP locale, notifiche del browser contro l'esempio dell'RFC 8291 e un servizio finto, e metodi dell'API attraverso il gateway, senza rete.

---

## 5. Prove dal PC: tunnel verso l'agente

L'agente ascolta solo su `127.0.0.1:3180` del server e sulla rete di Docker: dal PC ci si arriva con un tunnel SSH. Serve per interrogare l'agente a mano; il browser non passa di qui.

1. **Chiave SSH**: il server dev'essere raggiungibile da `ssh` **senza password** (chiave SSH). Comodo dare un alias in `~/.ssh/config` del PC, es.:
   ```
   Host oecopilot
       HostName  198.51.100.10
       User      mario
       IdentityFile ~/.ssh/id_ed25519
   ```
   Prova: `ssh oecopilot hostname` deve entrare senza chiedere nulla. L'OpenSSH di Windows rifiuta una chiave leggibile da altri utenti ("UNPROTECTED PRIVATE KEY FILE"): una chiave in una cartella sincronizzata (OneDrive) eredita spesso permessi in più, tienine una copia in `%USERPROFILE%\.ssh` e sistemala con `icacls <chiave> /inheritance:r /grant:r "%USERNAME%:R"`.

2. **Tunnel**, lasciato aperto: `ssh -N -L 13180:127.0.0.1:3180 oecopilot`.

3. **Chiamata all'agente**. La chiave dell'API sta in `.env` sul server: si legge in una variabile e non si stampa.
   ```bash
   CHIAVE=$(ssh oecopilot "sudo grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2-")
   curl -s -H "Authorization: Bearer $CHIAVE" http://127.0.0.1:13180/api/calendari
   ```

### Agente in TypeScript (in uso dal 12/10/2026)
`server/agente-ts/` è lo stesso agente scritto in TypeScript: stessa API (`/api/eventi`, `/api/calendari`, `/api/salute`, `POST /api/ics`), stessa chiave, stessi codici di stato e stessi testi di errore, lo stesso `calendar.db` letto in sola lettura e lo stesso `agent/fonti.json`. Dal 12/10/2026 serve il gateway (`AGENTE_URL=http://agente-ts:8180` in `.env`) e l'agente Python resta acceso come ritorno indietro. Sta dietro il profilo `agente-ts` di compose, quindi `docker compose up -d` e l'aggiornamento automatico (4.6) non lo avviano né lo ricostruiscono: dopo un aggiornamento che cambia `server/agente-ts/` si ricostruisce a mano (punto 1 qui sotto).

1. **Avviarlo accanto all'altro**, senza spostare niente: `sudo docker compose --profile agente-ts up -d --build agente-ts`. Risponde alle stesse chiamate su `127.0.0.1:3181` (tunnel come sopra, con `-L 13181:127.0.0.1:3181`). Gira come utente 10001, a file system in sola lettura e senza capacità; i file del calendario sono del gruppo 10019 (l'utente di Android che li scrive) e il gruppo entra con `group_add` nel file di compose (se Android cambia il numero, `sudo stat -c %g /opt/oe-copilot/telefono/data/data/com.android.providers.calendar/databases/calendar.db` lo dice e in `.env` si scrive `CALENDARIO_GID=<numero>`; senza il gruppo le caselle risultano in errore con "Permission denied" in `/api/eventi` e `/api/salute`).
2. **Farlo servire al gateway**: in `/opt/oe-copilot/.env` si scrive `AGENTE_URL=http://agente-ts:8180` e si ricrea il gateway, `sudo docker compose up -d gateway`. Il gateway ha un solo indirizzo dell'agente: vale per tutte le chiamate, `POST /api/ics` compreso.
3. **Tornare indietro**: si toglie la riga `AGENTE_URL` da `.env` e si ricrea il gateway. L'agente Python non si ferma mai.
4. **L'aggiornamento automatico (4.6)** ricrea e controlla sempre `agent` (porta 3180) e non tocca `agente-ts`: dopo un aggiornamento che cambia `server/agente-ts/` l'immagine si ricostruisce con il comando del punto 1.

Le risposte sono le stesse di Python byte per byte (provate su un calendario inventato in CI, su una copia di quello del telefono e su quello vivo, fianco a fianco), con tre differenze volute: un percorso con byte non ASCII non codificati (senza `%`) riceve 400, perché Node li rifiuta (il gateway codifica sempre); uno spazio dopo la chiave giusta in `Authorization` passa, perché Node lo toglie (Python risponde 401); un testo ICS senza a capo non viene mai preso per il percorso di un file (Python lo apre se esiste).

---

## 6. Il rilevatore degli accessi scaduti

Chiude il "falso verde" (vedi [ragionamenti](#1-i-ragionamenti-perché-è-fatto-così)).
`strumenti/rileva_accessi.py`, lanciato dal servizio ogni 2 giri (circa 70 secondi), scrive in `stato/accessi.json`
lo stato di ogni casella, che l'agente usa per il pallino — **senza mai leggere un token**:

- **Caselle Microsoft (Outlook), accesso:** la bandierina "login da rifare" (`needsReauth`) nel
  registro di Outlook. Quando c'è → pallino **rosso** "Accesso scaduto sul telefono". Segnale netto.
- **Caselle Microsoft (Outlook), freschezza:** Outlook non scrive da nessuna parte quando ha
  scaricato dal server. Con i servizi Google riceve le notifiche push (FCM), ma sono quelle della
  posta: una cancellazione o una modifica di un appuntamento non ne ha, e Outlook la scarica solo
  quando è davvero in primo piano, cioè a telefono sbloccato (11/10/2026: sette inviti cancellati
  alle 09:45 UTC arrivati sul telefono alle 11:42:29, appena sbloccato). Si misura quello che
  Outlook dice davvero: a ogni giro il servizio forza la sincronizzazione e Outlook risponde con
  una riga per casella in `calendarSync.log` (`isSyncingForAccount`); se una casella tace oltre
  `OUTLOOK_GIALLO_S` (10 min) è **gialla** ("Outlook non conferma la sincronizzazione della casella
  da N min"), oltre `OUTLOOK_ROSSO_S` (30 min) **rossa**; **gialla** anche se Outlook ha sospeso la
  casella. **Gialla** "Telefono bloccato: le cancellazioni e le modifiche fatte in Outlook arrivano
  in ritardo, sbloccalo dalla pagina Telefono" finché la schermata di blocco è su (`bloccato` in
  `stato/battito.json`, scritto dal servizio a ogni giro); un guasto vero (casella muta) pesa di
  più. Se le push non sono disponibili (`notifications.log`, "FCM push sync is unavailable") **e**
  Outlook non è in primo piano, **giallo** "non riceve le notifiche push".
- **Calendario Google (servizi Google del telefono):** da `dumpsys content`, l'esito
  dell'ultima sincronizzazione di `com.android.calendar` per l'account. Un fallimento isolato è
  la rete; senza un successo in mezzo oltre `GOOGLE_GIALLO_S` (15 min) è **giallo**, oltre
  `GOOGLE_ROSSO_S` (45 min) **rosso** ("probabile accesso scaduto, rifai il login Google"); la
  sincronizzazione che non parte da oltre 15 min è gialla. L'ora dell'ultimo successo si ricorda
  in `stato/google-sync.json`. L'adattatore dei contatti segna sempre `OTHER FAILURE
  internal-error`: non conta.

Le soglie si cambiano con le variabili d'ambiente `OUTLOOK_GIALLO_S`, `OUTLOOK_ROSSO_S`, `GOOGLE_GIALLO_S` e `GOOGLE_ROSSO_S` (nel servizio), e la cadenza del controllo con `ACCESSI_OGNI` (giri di sincronizzazione fra un controllo e l'altro).

Il rilevatore gira nel supervisore (4.5), in TypeScript (`supervisore/src/rilevatore/`): legge gli stessi file di Outlook (database e log, in sola lettura) e `dumpsys content` dal telefono con adb. `supervisore/test/differenziale.test.ts` lo confronta, a ogni modifica, con `rileva_accessi.py` su dati inventati (log con la rotazione, database, battito, `dumpsys content`, ora): `accessi.json` e `google-sync.json` devono uscire uguali byte per byte. L'unica differenza voluta: con byte non UTF-8 in `dumpsys content` il Python non dà verdetti per Google e il TypeScript sì. Il Python (`rileva_accessi.py`) resta nel repository come riferimento e come ritorno indietro finché il supervisore non ha girato a lungo senza problemi.

---

## 7. Manutenzione e diagnostica

- **Rifare un login scaduto**: dalla pagina `/telefono/` (4.8; finché è aperta la sincronizzazione sta in pausa), poi rifai il login nell'app Outlook o nelle impostazioni dell'account Google del telefono e chiudi la scheda.
- **Segni di vita** (sul server): `stato/battito.json` (il servizio è vivo? il telefono è acceso? Outlook è in primo piano?) e
  `stato/accessi.json` (stato di ogni casella).
- **Log dei servizi** (sul server): `docker logs oe-copilot-<servizio>` (`agent`, `gateway`, `schermo`, `caddy`, `supervisore`).
- **Vedere cosa fa l'agente**: `curl` su `http://127.0.0.1:3180/api/calendari` dal server (con la
  chiave, vedi 4.5), oppure dal PC col tunnel della sezione 5.
- **Aggiornare il codice del server**: con l'aggiornamento automatico (sezione 4.6) basta un push
  su `main`. A mano: `git pull`, ricopia `server/` in `/opt/oe-copilot` (mantieni `.env`, `agent/fonti.json`, `telefono/`, `stato/` e `ui-dati/`), `sudo docker compose up -d --build --remove-orphans` (toglie anche il container `oe-copilot-ui` del servizio Python che non c'è più), riavvia il servizio.
- **Prova dell'agente** (`server/strumenti/prova_agente_golden.py`, la lancia la Action): costruisce un `calendar.db` inventato come quello del telefono (serie con fusi e cambio dell'ora, occorrenze annullate e spostate, EXDATE, giorni interi, doppioni, inviti), avvia l'agente vero e confronta le sue risposte HTTP con quelle registrate in `server/strumenti/golden/agente.json`. Se una modifica all'agente o a una libreria cambia una risposta, la prova mostra la differenza; se la nuova risposta è quella giusta si riscrive il file con `docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/server/agent:/app:ro" -v "$PWD/server/strumenti:/prove" oe-copilot-agent python /prove/prova_agente_golden.py --aggiorna`. Le librerie dell'agente hanno le versioni fissate in `server/agent/requirements.txt`. La stessa prova controlla `POST /api/ics` su un corpus di calendari (`server/strumenti/golden/ics/*.ics`: festività, serie a New York attraverso il cambio dell'ora, esclusioni, giorni interi, orari UTC, testo non valido). Si registra in un container Linux (la VM: `docker run` come sopra con `-v "$PWD/server/strumenti:/prove"`): con `TZ=Europe/Rome` il runtime di Windows dà uno scarto fisso di un'ora e altera ogni risultato che passa da `astimezone()`.
- **Prove del gateway con le risposte registrate** (`server/web-ts/test/golden-lettura.test.ts` e `golden-scritture.test.ts`, `npm test`, le lancia la Action): i metodi dell'API del gateway rispondono a una sequenza di chiamate di lettura (`get_config`, `get_personalizzazione`, `get_calendars`, `get_events`, `get_disponibilita`) con un agente finto che ripete le risposte di `golden/agente.json` in ogni stato (fermo, in pausa, caselle in errore, irraggiungibile), su una cartella dei dati costruita da `golden/ui/fixture.json`, e a una sequenza di modifiche (interruttori, gruppi, alias, calendari da link, eventi personali, configurazione), dopo ognuna delle quali si controlla lo stato: `config.json`, `local_events.json` e le copie dei calendari ICS. Risposte e stato devono essere uguali a quelli registrati in `golden/ui-lettura.json` e `golden/ui-scritture.json`. Quei due file li registrò il servizio Python `ui` con la stessa sequenza (`server/strumenti/prova_ui_golden.py`); il servizio è stato tolto e l'ultimo commit che lo contiene è `142e789`. Da allora sono dati di prova come `fixture.json`: nessuno li rigenera, e una differenza è un comportamento del gateway che è cambiato, da leggere prima di toccare il file.
- **Prove del secondo fattore** (`server/web-ts/test/secondo-fattore.test.ts`, `passkey.test.ts`, `avvio.test.ts`, `qr.test.ts`, `npm test`, le lancia la Action): utenti di prova a caso in un database in memoria, mai la password di nessuno. Coprono l'attivazione, l'accesso con il codice (generato da un TOTP di RFC 6238 scritto nella prova, non dalla libreria che lo controlla), il codice sbagliato e il blocco, i codici di recupero a uso singolo, la disattivazione, `off` identico a prima (le strade non ci sono e, con il codice già attivato, basta la password), l'avvio con ogni valore di `AUTH_2FA` anche su un database fatto dall'immagine di prima, la cerimonia delle passkey con un autenticatore in software (accesso, firma falsa, origine o nome del sito sbagliati, sfida riusata, contatore che non cresce) e il QR, riletto da un lettore indipendente. Il passo *Gateway, accesso e cancello* della Action prova la modalità `off` sul container vero (log di avvio, tabelle, strade 404, valore sbagliato che ferma l'avvio) e l'avvio con `passkey` dall'immagine.
- **Backup**: i dati del telefono stanno in `/opt/oe-copilot/telefono/data` (contengono i login:
  trattali come un segreto). Uno snapshot coerente si fa con il telefono in `docker pause` per
  pochi secondi.
- **Stato dell'app** (interruttori, gruppi, alias, link ICS, colori, tema, eventi personali): sta in
  `ui-dati/gateway.db` e, come copia nel formato di sempre, in `ui-dati/config.json` e
  `ui-dati/local_events.json`, riscritti dopo ogni modifica: per un backup basta copiare `ui-dati/`. Per
  riportare lo stato a quello dei due file (dopo un ripristino, o dopo averli corretti a mano) si
  fermano il gateway, si svuotano le tabelle dello stato e si riavvia: alla prima apertura il gateway
  reimporta i file.
  ```bash
  docker stop oe-copilot-gateway
  docker run --rm --user 10001:10001 -v /opt/oe-copilot/ui-dati:/dati oe-copilot-gateway \
    node -e "new (require('better-sqlite3'))('/dati/gateway.db').exec('DROP TABLE stato_config; DROP TABLE eventi_locali; DROP TABLE stato_meta')"
  docker start oe-copilot-gateway
  ```

---

## 8. Sicurezza e privacy

- **Il telefono sbloccato ha un solo cancello: l'accesso al sito.** Il servizio tiene il telefono sbloccato dopo il PIN (4.5), il servizio `schermo` non ha password e `adb` non chiede conferme (`ro.adb.secure=0`): chi supera l'accesso davanti a `/telefono/` comanda un telefono già sbloccato, con dentro le caselle di lavoro. Il PIN non fa da secondo fattore. Per questo la password dell'interfaccia va lunga e casuale (almeno 12 caratteri); il blocco per account (5 errori, 15 minuti) e la sessione (30 giorni, cookie HttpOnly) sono le difese del sito. Il secondo fattore dell'accesso (4.7), spento di default, ci aggiunge il codice di un'app di autenticazione o le passkey.
- **Il PIN c'è perché lo chiede un criterio di sicurezza aziendale**, imposto a Outlook da una delle aziende le cui caselle sono sul telefono: senza una password del dispositivo Outlook rimuove la casella (4.5). Non si toglie, non si cambia e nessuno script lo digita.
- **Il tocco sulla barra di stato aggira apposta il blocco a 15 minuti** dello stesso criterio (`TOCCO_OGNI`, 4.5): serve perché un telefono bloccato non applica cancellazioni e modifiche. Il prezzo è quello del primo punto. Per tornare al blocco automatico: `TOCCO_OGNI=0`.
- **Nessun segreto nel repository**, né nei file tracciati né nella storia di git (controllato l'11/10/2026 con ricerche di chiavi, token e password). La chiave dell'API e le credenziali dell'interfaccia stanno in `.env` sul server (permessi 600) e i segreti dell'aggiornamento automatico nei secret della Action. Nomi, domini, indirizzi, calendari, sigle e schede stanno in `ui-dati/personalizzazione.json` e in `agent/fonti.json`: file che il repository non ha mai contenuto e che `.gitignore` esclude. `server/strumenti/controlla_dati_personali.py` cerca i dati della personalizzazione nei file tracciati e fallisce se ne trova (si lancia a mano prima di un push, con il percorso del file).
- **I login stanno nel telefono sul server**, non sul PC. I dati del telefono (`/opt/oe-copilot/telefono/data`) contengono i token degli accessi: proteggili come credenziali.
- **Niente esposto su internet** a parte l'interfaccia web: l'agente e l'adb ascoltano solo sul localhost del server; dal PC ci si arriva con un tunnel SSH (chiave SSH) e la chiave dell'API (sezione 5). Le eccezioni, facoltative, sono l'interfaccia nel browser (4.7) e lo schermo del telefono (4.8): HTTPS e una pagina di accesso davanti a una pagina che mostra i calendari e a una che comanda il telefono. Il servizio `schermo` in sé non è pubblicato e non ha password: ci arriva solo chi ha passato l'accesso del gateway.
- **I token non si leggono**: il rilevatore degli accessi guarda solo bandierine ed età di sincronizzazione, mai il contenuto di un token.
- Non estrarre token/cookie dai browser e non usare client ID di altre app per aggirare un consenso.
