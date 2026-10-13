# OE Copilot, guida completa

Guida per installare OE Copilot: il **server Linux** (la "VM") che legge i calendari e li mostra in un browser. Spiega anche *perché* è fatto così, perché le scelte non sono ovvie e senza il ragionamento è facile rifarle male.

> In breve: OE Copilot unisce in un calendario unico tutte le caselle (aziendali e personali), con un colore per azienda. Le caselle le legge un agente sul server, da un telefono Android virtuale in cui gli accessi si fanno una volta sola. Si vedono **nel browser**: il server serve la pagina dietro HTTPS e una pagina di accesso (sezione 4.7), e sul PC non si installa niente (un client facoltativo per Windows e Android, in `app/`, apre lo stesso sito: sezione 3). L'app per Windows in Python è stata ritirata l'11/10/2026 (sezione 3).
>
> Sul PC Outlook non serve: gli accessi stanno sul server.

Indice:
1. [I ragionamenti (perché è fatto così)](#1-i-ragionamenti-perché-è-fatto-così)
2. [Architettura](#2-architettura)
3. [L'app per Windows (ritirata)](#3-lapp-per-windows-ritirata)
   - [Il client in app/ (Windows e Android)](#il-client-in-app-windows-e-android)
4. [Installazione sul server](#4-installazione-sul-server)
   - [Messa in servizio per un altro server](#messa-in-servizio-per-un-altro-server)
   - [4.1 Requisiti dell'host](#41-requisiti-dellhost)
   - [4.2 Prendere il codice](#42-prendere-il-codice)
   - [4.3 Avviare lo stack](#43-avviare-lo-stack)
   - [4.4 Configurare il telefono virtuale](#44-configurare-il-telefono-virtuale)
   - [4.5 Il servizio di sincronizzazione](#45-il-servizio-di-sincronizzazione)
   - [4.6 Aggiornamento automatico (facoltativo)](#46-aggiornamento-automatico-facoltativo)
     - [Quando un container si ricrea](#quando-un-container-si-ricrea)
     - [Il Caddy dopo l'aggiornamento](#il-caddy-dopo-laggiornamento)
     - [Tornare indietro solo con il Caddy](#tornare-indietro-solo-con-il-caddy)
     - [Utente della Action e secret del repository](#utente-della-action-e-secret-del-repository)
   - [4.7 L'interfaccia nel browser (facoltativa)](#47-linterfaccia-nel-browser-facoltativa)
     - [Secondo fattore dell'accesso](#secondo-fattore-dellaccesso-facoltativo)
     - [Gruppi di calendari](#gruppi-di-calendari)
     - [Disponibilità pubblica](#disponibilità-pubblica)
     - [Loghi delle aziende](#loghi-delle-aziende)
     - [I calendari da link e la guardia di rete](#i-calendari-da-link-e-la-guardia-di-rete)
     - [Personalizzazione](#personalizzazione)
   - [4.8 Lo schermo del telefono nel browser (facoltativo)](#48-lo-schermo-del-telefono-nel-browser-facoltativo)
   - [4.9 I promemoria sul server (facoltativo)](#49-i-promemoria-sul-server-facoltativo)
   - [4.10 Condividere il calendario con un altro OE Copilot (facoltativo)](#410-condividere-il-calendario-con-un-altro-oe-copilot-facoltativo)
5. [Prove dal PC: tunnel verso l'agente](#5-prove-dal-pc-tunnel-verso-lagente)
   - [L'agente (TypeScript)](#lagente-typescript)
6. [Il rilevatore degli accessi scaduti](#6-il-rilevatore-degli-accessi-scaduti)
7. [Manutenzione e diagnostica](#7-manutenzione-e-diagnostica)
   - [Diagnosi rapida](#diagnosi-rapida)
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

Il supervisore (`oe-copilot-supervisore`) gira sul server e a ogni giro (circa 35 secondi: 30 di pausa più il lavoro del giro) forza i lavori di sincronizzazione, porta Outlook sulla schermata principale e tiene sveglio il telefono. Così l'aggiornamento è di ~20-30 s invece di 15 min, finché Outlook è in primo piano.

**Quando un'occorrenza è annullata.** Per l'agente (`annullata()` in `server/agente-ts/src/android.ts`) un'occorrenza di una serie è annullata se la riga di `calendar.db` che la sostituisce ha `eventStatus` 2 (`STATUS_CANCELED`), con una eccezione. Outlook scrive quello stato anche per le serie copiate da un flusso nel calendario di chi le legge (`isOrganizer` 1, nessun invitato: `hasAttendeeData` 0), per tutte le loro occorrenze future, mentre le mostra e non le dà per annullate: nel suo log (`app_logs/calendarSyncToNativeCRUD.log`) la riga porta `isCanceled` ma non `outlook_isCancelled`, che invece c'è per un annullamento vero. Verificato il 13/10/2026: la riunione di Tesco delle 15:30 ("Open shift weekly update", serie settimanale) era nascosta da questo stato e in Outlook risultava normale; sul calendario vero 255 righe annullate su 265 con una serie erano di quel tipo, e 50 occorrenze del mese dopo erano nascoste (otto riunioni, tra cui lo stand-up di NPO e la daily pipeline di CGM). Quelle righe ora non annullano più l'occorrenza. Un annullamento vero ha un invitato (`isOrganizer` 0 e `hasAttendeeData` 1) oppure il titolo "Canceled:" (o "Annullato:"), che il gateway toglie da pagina, disponibilità, condivisione e promemoria. Una riga annullata che non sostituisce una serie resta annullata in ogni caso. Quelle righe hanno spesso la fine uguale all'inizio (durata zero): per loro vale la durata della serie (il suo `dtend` meno `dtstart`, o `duration`), se c'è.

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
- **agent** = l'agente (`server/agente-ts/`, TypeScript, container `oe-copilot-agent`): piccolo server HTTP che legge quel calendario e lo espone con un'API a chiave, in ascolto solo su `127.0.0.1:3180` del server. Espande anche le ricorrenze dei calendari ICS (link) che il gateway gli manda (`POST /api/ics`).
- **supervisore** (`oe-copilot-supervisore`, sempre acceso: nessun profilo di compose) = tiene aggiornato il telefono, rileva gli accessi scaduti e scrive i "segni di vita" (`battito.json`, `accessi.json`).
- **interfaccia nel browser** (4.7) = `caddy-ui` (HTTPS) e `gateway` (pagina di accesso, pagina in TypeScript, loghi e tutta la logica dell'app: eventi, calendari, configurazione, disponibilità, interruttori, gruppi, alias, link ICS, eventi personali; lo stato sta nel suo database). Il gateway chiede gli eventi all'agent sulla rete di Docker. Il vecchio servizio Python `ui` non c'è più: l'ultimo commit che lo contiene è `142e789`.
- **schermo** (4.8) = il telefono mostrato e comandato nel browser, dietro lo stesso accesso.
- **client Windows e Android** (`app/`, facoltativo) = una finestra sull'interfaccia nel browser, con le notifiche delle riunioni (sezione 3).

Il codice del server sta nella cartella **`server/`** del repository.

Fino al passaggio del 12/10/2026 l'agente era in Python (`server/agent/`) e la sincronizzazione era un servizio systemd, `oe-copilot-sincronizza`, con uno script bash (`strumenti/sincronizza_telefono.sh`) e un rilevatore degli accessi in Python (`strumenti/rileva_accessi.py`). L'agente in TypeScript e il supervisore li hanno sostituiti dopo essere stati provati uguali (sezioni 5 e 6), e il codice vecchio è stato tolto dal repository: sta nella cronologia di git (lo contiene, per esempio, il commit `00307be`). Tornare all'implementazione di prima vuol dire un `git revert` del commit che l'ha tolta (`4d26c48`, che tocca anche `docker-compose.yml`, il workflow e `aggiorna_server.sh`; i commit venuti dopo hanno cambiato ancora il workflow e lo script, quindi il revert può dare conflitti da risolvere) e un nuovo deploy.

---

## 3. L'app per Windows (ritirata)

L'app per Windows (Python, pywebview e Outlook classico) è stata ritirata l'11/10/2026: leggeva le caselle dal server con un tunnel SSH e ci aggiungeva i promemoria di Windows e Telegram, l'avvio con Windows e il pulsante *Apri in Outlook*. L'interfaccia nel browser (4.7) fa lo stesso lavoro di lettura senza installare niente sul PC; il codice dell'app per Windows resta nella storia di git (ultimo commit con l'app: `360c5c7`).

Cosa è cambiato:
- **Promemoria**: quelli dell'app partivano solo finché l'app era aperta. Ora li può mandare il server su Telegram o nelle notifiche del browser, anche a pagina chiusa (4.9, spenti di norma); il client in `app/` avvisa da sé delle riunioni in arrivo finché è in esecuzione (più sotto).
- **Apri in Outlook**: non c'è. Nel browser *Apri in Teams* e *Partecipa* aprono il link della riunione.
- **Estensioni personali** (`personale/`): il meccanismo non esiste più. Una scheda Timesheet con `azione` e senza `url` non compare; con `url` apre il gestionale.
- **Tunnel e chiave dell'agente sul PC** (`agente_remoto` in `personalizzazione.json`, `agente_chiave.bin`): non servono più. Il file locale, se c'è ancora, si può cancellare.

### Il client in app/ (Windows e Android)

Il successore è il client in `app/` (Tauri v2): una finestra sull'interfaccia nel browser, per Windows e per Android. Non contiene l'interfaccia: mostra il sito del server con il suo accesso, e cambia solo quando serve una funzione nativa.
- **Windows**: icona nell'area di notifica, istanza unica, avvio con Windows e notifiche di Windows per le riunioni in arrivo (di norma 10 minuti prima, o gli anticipi scelti nei promemoria del server). È costruito e provato sul PC; l'installer non è firmato.
- **Android**: la stessa finestra (pacchetto `com.oecopilot.app`), con la notifica di sistema per le riunioni; il permesso di Android 13 si chiede una volta. L'APK è costruito, firmato e provato su un emulatore, mai su un telefono vero.
- Le notifiche sono quelle normali del sistema, con titolo, ora e luogo, senza pulsanti né link per entrare; un avviso in stile Teams non c'è. Partono solo finché l'app è in esecuzione: con l'app chiusa i promemoria li manda il server (4.9).
- L'indirizzo del sito si fissa alla costruzione (`OE_URL`): cambiare sito vuol dire ricostruire. Come si costruisce, si firma, si installa e si prova, e cosa non c'è (*Apri in Outlook*, aggiornamento automatico, firma del codice, uso offline), sta in `app/README.md`.

Il login si fa una volta: il cookie di sessione dura 30 giorni e resta nel profilo dell'app anche dopo averla chiusa; quando non vale più (password cambiata, utente tolto, 30 giorni passati) il sito riporta da solo alla pagina di accesso. L'installer va eseguito da Windows e non da dentro un'app impacchettata (MSIX), che sposta i dati in una cartella virtuale (sezione "Accesso e sessione" di `app/README.md`).

---

## 4. Installazione sul server

### Messa in servizio per un altro server

Lista per chi mette in piedi una propria istanza, per esempio quella di un amico che riceve il calendario condiviso (4.10). L'istanza si aggiorna dalla Action del suo repository. La prima installazione è a mano: il repository non ha uno script per farla, e `aggiorna_server.sh` si ferma se mancano `.env` o `agent/fonti.json`. Ogni punto rimanda alla sezione che ha i comandi.

1. **Prerequisiti.** Un host Linux ARM64 con Docker e il plugin `compose`, con `rsync` (l'Action copia il codice con rsync) e `curl`, e il modulo del kernel `binder_linux` (4.1). Un repository GitHub proprio con una copia di questo repository e le Action attive. Un nome per il sito che arrivi al server sulla 443 (HTTPS, 4.7). Sul PC servono `ssh` e `gh`. Il gateway (`server/web-ts/src/lib/core/fuso.ts`) e l'agente (`TZ` nel suo Dockerfile) usano il fuso `Europe/Rome`, e `prepara_telefono.sh` imposta lo stesso fuso sul telefono a ogni aggiornamento: per un altro fuso il repository non ha una variabile.
2. **L'APK di Outlook.** Il repository non lo contiene: pesa più del limite di GitHub per un file, e nel repository c'è solo l'impronta (`server/apk/SHA256SUMS`). Nel proprio repository va creata la release `apk` (`gh release create apk <file> --repo <proprietario>/<repository>`) con il file `Outlook-arm64-v8a-<versione>.apk`, e la riga di `sha256sum <file>` va in `server/apk/SHA256SUMS` (4.4). L'Action scarica la release a ogni aggiornamento e si ferma se manca o se l'impronta non coincide. Dove procurarsi l'APK il repository non lo dice.
3. **Codice, utente e cartelle.** Il codice in `/opt/oe-copilot` (4.2), l'utente `oecopilot` con i suoi proprietari (4.6) e le cartelle dei dati, ognuna del proprietario che serve ai container: `sudo install -d -o 10080 -g 10080 -m 755 /opt/oe-copilot/stato`, `sudo install -d -o 10001 -g 10001 -m 755 /opt/oe-copilot/stato/schermo` e `sudo install -d -o 10001 -g 10001 -m 700 /opt/oe-copilot/ui-dati`. Senza `stato/schermo` scrivibile l'aggiornamento diventa rosso (4.6).
4. **`.env`**, dell'utente `oecopilot`, permessi 600: `UI_HOST=<nome del sito>` (senza, l'interfaccia nel browser non parte) e, se servono, `AUTH_2FA`, `CALENDARIO_GID` e i valori dei promemoria (modello in `.env.esempio`). `API_KEY`, `ADMIN_EMAIL`, `ADMIN_PASSWORD` e `BETTER_AUTH_SECRET` non vanno in `.env`: con la Action arrivano dai secret (4.7).
5. **`agent/fonti.json`.** Deve esistere prima del primo aggiornamento. Si parte da `agent/fonti.esempio.json` e si mettono le caselle vere dopo i login (4.2). Finché non ci sono, le voci del modello non portano a caselle e l'agente le dà in errore.
6. **Secret del repository**, otto, con `gh secret set <NOME> --repo <proprietario>/<repository>`: `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY` e `DEPLOY_KNOWN_HOSTS` per il collegamento SSH (la chiave pubblica va in `authorized_keys` di `oecopilot`, 4.6), e `ADMIN_EMAIL`, `ADMIN_PASSWORD` (almeno 12 caratteri, senza `$`), `API_KEY` e `BETTER_AUTH_SECRET` (`openssl rand -hex 32` per tutte e due) per l'agente e l'accesso al sito.
7. **Primo aggiornamento.** Un push su `main`, oppure `gh workflow run Server --repo <proprietario>/<repository> --ref main`. Avvia il telefono (la prima volta costruisce l'immagine, e `prepara_telefono.sh` installa Outlook), poi agente, supervisore, interfaccia e schermo. Finché Caddy non ha il certificato il controllo del Caddy può fallire: si rilancia dopo qualche minuto (4.6). A mano non si lancia `docker compose up` (4.3, 4.7).
8. **Accesso e login sul telefono.** `https://<UI_HOST>/login` con `ADMIN_EMAIL` e `ADMIN_PASSWORD`. Dal pulsante *Telefono* (`/telefono/`, 4.8) si fanno i login nelle app del telefono e si accende *Sincronizza calendari* per ogni casella (4.4). Poi si scrivono le caselle in `agent/fonti.json` e si riavvia l'agente con `sudo docker compose restart agent`.
9. **PIN del telefono.** `prepara_telefono.sh` non ne imposta nessuno. Serve solo se una casella impone una password del dispositivo: Outlook mostra *Imposta la password del dispositivo* e senza rimuove la casella. Il PIN (almeno 4 cifre, senza ripetizioni né sequenze) lo digita sempre l'utente, si imposta come in 4.5 e non si toglie. Dopo il PIN il supervisore tiene il telefono sbloccato, quindi l'unico cancello è l'accesso al sito (sezione 8).
10. **Il resto, quando serve.** Secondo fattore: `AUTH_2FA` in `.env` e un nuovo aggiornamento (4.7). Promemoria su Telegram: `TELEGRAM_BOT_TOKEN` e `TELEGRAM_CHAT_ID` in `.env` e un nuovo aggiornamento (4.9). Link di condivisione: *Impostazioni → Condivisione* (4.10). Loghi: *Gestisci calendari*, pulsante *Logo* (4.7; la cartella `ui-dati/logos` la crea il gateway al primo salvataggio). Per i casi che non vanno: *Diagnosi rapida*, sezione 7.

### 4.1 Requisiti dell'host
- Un host **Linux** con **Docker** (e il plugin `compose`).
- Kernel con il modulo **`binder_linux`** (serve a redroid). Verifica:
  ```bash
  grep -E 'CONFIG_ANDROID_BINDER_IPC|CONFIG_ANDROID_BINDERFS' "/boot/config-$(uname -r)"
  # devono essere =y o =m
  ```
  Provato su **Ubuntu 24.04 ARM64** (i kernel `-oracle` hanno `binder_linux` come modulo). Su x86-64
  non è provato e il repository non ha i pezzi per x86-64: i servizi Google di
  `server/immagine-telefono/` e l'APK di Outlook elencato in `server/apk/SHA256SUMS` sono per
  arm64-v8a. Su alcuni kernel "cloud" il modulo manca: in quel caso
  serve un kernel che lo includa (es. `linux-image-generic`/`-oracle`) o `linux-modules-extra`.
- Risorse: il telefono usa ~1,5 GB di RAM e ~2 GB di disco; conta qualche GB in più per immagini
  Docker e app. Una VM da 2-4 GB di RAM basta.

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
`/opt/oe-copilot` non deve esistere ancora: se esiste, `cp -a` crea `/opt/oe-copilot/server` al suo interno.

Crea i due file di configurazione (restano solo sul server, non nel repository):
```bash
# chiave dell'API: lunga e casuale
printf 'API_KEY=%s\n' "$(openssl rand -hex 32)" | sudo tee .env >/dev/null
sudo chmod 600 .env

# elenco delle caselle da leggere: copia l'esempio e mettici le tue
sudo cp agent/fonti.esempio.json agent/fonti.json
sudoedit agent/fonti.json
```
Questa `.env` è quella di una prima installazione a mano. Con l'aggiornamento automatico (4.6) l'agente prende la chiave dal secret `API_KEY` della Action: la riga si può togliere, e `.env` tiene `UI_HOST` e le altre variabili (4.7).

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
Questo passo è la prima installazione a mano. Su un server aggiornato dalla Action (4.6) niente si avvia a mano: i secret non stanno in `.env`, e un `docker compose up` ricrea l'agente con `API_KEY` vuota e il gateway senza i suoi valori (4.7).

```bash
sudo docker compose up -d telefono          # avvia 'telefono' (redroid)
sudo docker exec oe-copilot-telefono getprop sys.boot_completed    # ripeti finché risponde 1: il telefono è avviato
sudo docker compose up -d agent             # avvia 'agent', solo a telefono avviato
sudo docker ps                   # devono essere "Up" entrambi
```
Il primo avvio scarica le immagini. Se il telefono non parte, quasi sempre manca il modulo binder
(vedi 4.1). L'agente si avvia solo a telefono avviato: monta la cartella del calendario di Android e, se non esiste ancora, Docker la crea vuota e di root al posto di Android. Compose prende `API_KEY` da `.env` (4.2) o dall'ambiente della shell: senza, l'agente non parte (sulla chiave e sui secret: 4.7).

### 4.4 Configurare il telefono virtuale
**Impostazioni e app.** Fuso orario, lingua, schermo sempre acceso, l'app Outlook, niente
risparmio batteria per Outlook e il permesso sul calendario del sincronizzatore di Google li mette
lo script, senza toccare lo schermo:
```bash
sudo /opt/oe-copilot/strumenti/prepara_telefono.sh
# altro fuso o altra lingua: sudo FUSO=Europe/London LINGUA=en-GB .../prepara_telefono.sh
```
Si può rilanciare quanto si vuole (fa solo quello che manca) e l'aggiornamento automatico della
[sezione 4.6](#46-aggiornamento-automatico-facoltativo) lo lancia a ogni aggiornamento: un telefono ricreato
da zero torna a posto da solo, restano da rifare solo i login.

L'app Outlook esiste solo sul Play Store, quindi l'APK
(**arm64-v8a**: redroid qui è 64-bit, gli APK armeabi-v7a falliscono) sta fra i file della release
[`apk`](../../releases/tag/apk) del repository. Pesa più dei 100 MB che GitHub accetta per un file
nel repository, per questo è lì e non fra i sorgenti. L'aggiornamento automatico lo scarica e lo
porta in `/opt/oe-copilot/apk/`; a mano:
```bash
gh release download apk --repo <proprietario>/OE-Copilot --pattern 'Outlook-*.apk' --dir /opt/oe-copilot/apk
```
Lo script installa solo l'APK elencato in `apk/SHA256SUMS` (che è nel repository) e solo se
l'impronta coincide. Che sia davvero l'app di Microsoft lo dice la firma, che Android controlla
all'installazione: il certificato ha SHA-1 (base64) `fcg80qvoM1YMKJZibjBwQcDfOno=` (è pubblico, lo
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
   l'agente: `sudo docker compose restart agent`.

> I login (password e MFA) li fai **tu**, dallo schermo del telefono.

### 4.5 Il servizio di sincronizzazione
Il servizio di sincronizzazione è il supervisore: tiene aggiornato il telefono e scrive i segni di vita. È il servizio `supervisore` di compose (`supervisore/`, Node.js e TypeScript, adb sulla rete di compose). Lo ricrea l'aggiornamento automatico (4.6) a ogni deploy; a mano si avvia con `docker compose up -d --build --no-deps supervisore`. Il vecchio servizio systemd `oe-copilot-sincronizza` (uno script bash con il rilevatore in Python) non c'è più: è stato tolto dal repository e sta nella cronologia di git.
Un giro dura circa 35 secondi (30 di pausa, `OGNI`, più il lavoro del giro) e fa questo:
- forza i lavori di sincronizzazione del calendario di Outlook per ogni casella e, ogni 4 giri (`GOOGLE_OGNI`), quelli di Google;
- se lo schermo è spento e nessuna pagina del telefono è aperta, lo riaccende; poi Outlook esce dallo schermo (si porta davanti la schermata iniziale di Android) e subito dopo **torna sulla schermata principale**. Con quel ritorno Outlook ha scaricato le cancellazioni (11/10/2026, 11:42:26 UTC); che serva anche restando sempre davanti non è provato;
- scrive `stato/battito.json` (`quando`, `pausa`, `telefono`, `outlook_davanti`, `bloccato`);
- ogni 2 giri (`ACCESSI_OGNI`, circa 70 secondi) lancia il **rilevatore degli accessi** (sezione 6);
- ogni 10 giri (`TOCCO_OGNI`, circa 6 minuti, `0` spegne) manda un tocco sulla barra di stato a schermo acceso.

Con il file `PAUSA` (`telefono/PAUSA`, oppure `stato/schermo/PAUSA` finché una pagina `/telefono/` è aperta, 4.8) il servizio non porta Outlook in primo piano e forza solo le sincronizzazioni di Android.

**Telefono sbloccato.** Outlook applica una cancellazione o una modifica solo quando è davvero davanti, cioè a telefono sbloccato; a telefono bloccato arrivano solo le push della posta. Il criterio del dispositivo di Outlook (sezione 1) blocca il telefono 15 minuti dopo l'ultimo input, e il tocco sulla barra di stato rimanda il blocco: dopo il PIN il telefono resta sbloccato, e chi arriva alla pagina del telefono (dopo l'accesso) lo trova sbloccato. Per tornare al blocco dopo 15 minuti si imposta `TOCCO_OGNI=0` nel servizio `supervisore` di `docker-compose.yml`, nel repository, e si ridistribuisce.

**Il PIN non si può togliere.** Il criterio chiede una password del dispositivo: toglierla (`cmd lock_settings clear --old <PIN>` dal server) l'11/10/2026 ha fatto comparire in Outlook la schermata "Imposta la password del dispositivo", con l'avviso che altrimenti la casella viene rimossa. Il PIN si rimette con `cmd lock_settings set-pin <PIN>` dal server (le schermate del PIN in Impostazioni escono nere nella pagina). Lo sblocco lo digita sempre l'utente: se lo schermo è spento il servizio lo riaccende, sulla schermata di blocco, e non manda altro.

**Il supervisore: utente e cartelle.** `supervisore/` (Node.js, TypeScript) fa il lavoro che faceva lo script bash con il rilevatore in Python: stessi giri, stesse variabili d'ambiente (`OGNI`, `ACCESSI_OGNI`, `GOOGLE_OGNI`, `TOCCO_OGNI`, `PAUSA_SCHERMO_SCADE`, le soglie della sezione 6) e stessi file in `stato/`. Parla col telefono con adb sulla rete di compose (`telefono:5555`, come il servizio schermo) invece di `docker exec`, quindi non ha `docker.sock` e non gira come root. È il servizio `supervisore` di compose, senza profilo: un `docker compose up` lo avvia come gli altri servizi e l'aggiornamento automatico (4.6) lo ricrea a ogni deploy. Gira come l'utente 10080, quello di Outlook sul telefono (`u0_a80`): database e log di Outlook sono leggibili solo da lui e da root (un container senza capacità, root compreso, non li legge) e adb non li raggiunge (parla come l'utente `shell`), quindi restano file del server, montati in sola lettura. `stato/` deve essere suo, `stato/schermo` resta dell'utente 10001:
```bash
cd /opt/oe-copilot
sudo chown 10080:10080 stato
sudo install -d -o 10001 -g 10001 -m 755 stato/schermo       # solo se manca (di solito c'è già)
docker compose up -d --build --no-deps supervisore
docker inspect -f '{{.State.Health.Status}}' oe-copilot-supervisore       # healthy entro circa un minuto
cat stato/battito.json; stat -c '%y' stato/accessi.json                    # battito ogni 35 secondi, accessi ogni 70
```
L'aggiornamento automatico (4.6) ricrea il supervisore a ogni deploy e controlla con `docker inspect` che giri. Il tocco sulla barra di stato parte da adb come utente `shell` e non da root, e conta come attività dell'utente: lo si è visto il 12/10/2026 al passaggio, l'età dell'ultima attività (`dumpsys power`, `mLastUserActivityTime`) si azzera a ogni tocco (giro 0, giro 10) e il telefono è rimasto sbloccato oltre i 15 minuti dall'ultimo tocco del vecchio servizio. Per ricontrollarlo: `dumpsys power` prima e dopo un giro con il tocco.

Verifica che l'agente risponda (dal server):
```bash
curl -s -H "Authorization: Bearer $(sudo docker exec oe-copilot-agent printenv API_KEY)" \
  http://127.0.0.1:3180/api/calendari | python3 -m json.tool | head
```

### 4.6 Aggiornamento automatico (facoltativo)
La GitHub Action `.github/workflows/server.yml` porta `server/` sul server a ogni push su `main` che tocca `server/` o il workflow stesso (l'elenco è in `paths` del workflow): controlla la sintassi e le prove (compresi il confronto dell'agente con le risposte registrate in `server/strumenti/golden/` e le prove e la build del gateway, sezione 7), copia il codice con rsync (`--delete`) e lancia `strumenti/aggiorna_server.sh`, che rimette a posto impostazioni e app del telefono (`prepara_telefono.sh`, con l'APK di Outlook preso dalla release `apk`), ricostruisce l'agente e il supervisore, senza toccare il telefono (l'agente si ricrea se cambia la sua immagine o la sua configurazione, il supervisore a ogni aggiornamento), e verifica che l'agente risponda, che il supervisore giri e che il battito sia fresco. Se un passo non riesce (avvio del telefono, preparazione del telefono, avvio dell'agente) o la verifica fallisce, rimette la versione di prima. `.env`, `agent/fonti.json`, `telefono/`, `stato/`, `ui-dati/` e `.versione` non vengono toccati dalla copia (`.versione` lo scrive lo script, con lo SHA distribuito). Se `.env` o `agent/fonti.json` mancano lo script si ferma subito: la prima installazione è a mano. Il telefono si ricrea solo se cambia la sua parte di `docker-compose.yml` o la sua immagine (`immagine-telefono/`): un riavvio inutile fermerebbe le sincronizzazioni per un paio di minuti. A fine aggiornamento parte l'interfaccia nel browser (4.7), solo se in `.env` c'è `UI_HOST`, e parte lo schermo del telefono (4.8), sempre: per lo schermo la cartella `stato/schermo` deve essere scrivibile dal servizio. Un loro guasto rende rossa la Action ma non riporta indietro l'agente né il telefono. Dopo aver avviato il gateway nuovo l'aggiornamento toglie, se c'è ancora, il container `oe-copilot-ui` del servizio Python che non esiste più (e la sua immagine). Toglie allo stesso modo il container `oe-copilot-agente-ts` (e la sua immagine), l'agente in TypeScript di prima, ma solo se il gateway sano non lo usa (`AGENTE_URL` vuoto o `http://agent:8180`): con un altro `AGENTE_URL` in `.env` lo lascia e scrive il motivo nel log. Un push che tocca solo file fuori da `server/` e dal workflow (per esempio `GUIDA.md` o `README.md`) non lancia niente, e `.versione` resta indietro.

#### Quando un container si ricrea

`docker compose up -d --build` ricrea un servizio solo se è cambiato qualcosa: la sua parte di `docker-compose.yml`, i valori d'ambiente che compose gli passa (i secret compresi) o il codice da cui si costruisce l'immagine. Una ricostruzione senza modifiche lascia il container com'è, con lo stesso `StartedAt`; una modifica vera dei sorgenti lo ricrea e il codice nuovo gira (provato il 12/10/2026 su un progetto compose di prova sul server). Per questo l'agente non ha bisogno di `--force-recreate`: ce l'ha solo il supervisore, a ogni aggiornamento. Un commento cambiato in un Dockerfile, che non cambia l'immagine, non ricrea niente. Per sapere se un servizio gira con il codice nuovo si confrontano prima e dopo `Created` e `StartedAt` del container (`docker inspect -f '{{.Created}} {{.State.StartedAt}}' oe-copilot-agent`) e `.versione`. Gli ID delle immagini (`docker images`, `.Image` del container, l'etichetta `com.docker.compose.image`) non servono a questo: con l'archivio di immagini containerd differiscono per ogni servizio, perché uno è il digest dell'indice e l'altro quello della configurazione.

#### Il Caddy dopo l'aggiornamento

Dopo i controlli del gateway lo script controlla anche il Caddy: `docker compose up` dà 0 anche se il container si riavvia in continuazione, e un Caddy che non parte con un gateway sano lascerebbe il sito spento dietro sslh con la Action verde. Fa una richiesta HTTPS alla porta pubblicata (`curl -k --resolve <UI_HOST>:4445:127.0.0.1 https://<UI_HOST>:4445/login`, con `UI_HOST` preso da `.env`), fino a 15 tentativi a due secondi l'uno dall'altro, che deve dare 200, e controlla che `RestartCount` di `oe-copilot-caddy` sia 0. Se non è così scrive su stderr il codice di risposta, il numero di riavvii e le ultime 40 righe del log di `oe-copilot-caddy`, e rende rossa la Action: come per il gateway non torna indietro niente, agente e telefono restano come sono. `-k` accetta un certificato non fidato, non la mancanza del certificato: alla prima installazione, finché Caddy non ha ottenuto il suo, la risposta non arriva e il controllo fallisce; il sito parte da sé appena il certificato c'è, e basta rilanciare la Action dopo qualche minuto. Un `RestartCount` diverso da zero resta finché il container non si ricrea: si legge `docker logs oe-copilot-caddy` per capire perché è caduto, poi `docker compose up -d --force-recreate --no-deps caddy-ui`.

#### Tornare indietro solo con il Caddy

`aggiorna_server.sh salva` tiene il codice di prima in `/var/tmp/oe-copilot-prima.tgz`. Se un Caddyfile nuovo non parte e il resto va bene, si rimette solo quello: `cd /opt/oe-copilot && tar -xzf /var/tmp/oe-copilot-prima.tgz ./caddy-ui && docker compose up -d --no-deps --build caddy-ui`. `--no-deps` è obbligatorio: senza, compose rivaluta anche il gateway e lo ricrea senza i valori che in una shell aperta a mano mancano (`ADMIN_EMAIL`, `ADMIN_PASSWORD`, `API_KEY` e `BETTER_AUTH_SECRET` possono arrivare dalla Action sullo standard input e non stare in `.env`).

#### Utente della Action e secret del repository

Sul server serve, una volta sola, un utente dedicato che entra solo con una chiave SSH:
```bash
sudo useradd -m -s /bin/bash -G docker oecopilot
# il codice è dell'utente della Action; telefono/ resta di root (lo crea Docker), stato/ è dell'utente 10080 (quello del supervisore, 4.5) con stato/schermo del 10001, ui-dati/ dell'utente 10001
sudo chown oecopilot:oecopilot /opt/oe-copilot /opt/oe-copilot/.env
cd /opt/oe-copilot && sudo chown -R oecopilot:oecopilot agent agente-ts apk caddy-ui immagine-telefono schermo-web strumenti supervisore web-ts docker-compose.yml .env.esempio
```
Con il supervisore l'utente della Action non ha bisogno di `sudo`: prima aveva due comandi `systemctl` in `/etc/sudoers.d/oe-copilot` (`daemon-reload` e il riavvio del servizio di sincronizzazione). Un server messo in servizio prima del 12/10/2026 può avere ancora quel file: non serve più e si toglie con `sudo rm /etc/sudoers.d/oe-copilot`.

La chiave pubblica della Action va in `/home/oecopilot/.ssh/authorized_keys`. Nel repository
servono otto secret (`gh secret set <NOME> --repo <proprietario>/<repository>`): quattro per il
collegamento, `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY` (la chiave privata) e `DEPLOY_KNOWN_HOSTS`
(la riga di `ssh-keyscan -t ed25519 <server>`), e quattro per l'agente e l'accesso al sito, `ADMIN_EMAIL`,
`ADMIN_PASSWORD`, `API_KEY` e `BETTER_AUTH_SECRET` (4.7). Senza i secret `DEPLOY_*`
l'aggiornamento fallisce al collegamento SSH: chi non ha un server disattiva la Action.

### 4.7 L'interfaccia nel browser (facoltativa)
Il calendario unico (caselle, colori, pallini di stato e filtri) con una pagina in TypeScript,
servita dal server a un browser qualunque, con HTTPS e una pagina di accesso. Gli interruttori
(calendari, aziende, alias, calendari da link ICS, eventi personali) restano sul server. I promemoria
delle riunioni sono facoltativi e li manda il server (4.9). Non c'è *Apri in Outlook*; i link delle
riunioni si aprono nel browser.

Come è fatta, in `server/docker-compose.yml`:
- **`gateway`** (`server/web-ts/`, Next.js e Better Auth): la pagina di accesso, la pagina in
  TypeScript, i metodi dell'API e i loghi. Senza sessione una pagina rimanda a `/login?next=...` e
  l'API (`/api/*`) risponde 401 in JSON (restano aperti `/login` con `/api/auth/*`, `/salute`, la
  disponibilità pubblica, i link `/condividi/<testo segreto>.ics` di 4.10 e i file statici come `/sw.js`);
  con la sessione il gateway risponde da sé: i metodi
  (`POST /api/<metodo>`, `server/web-ts/src/lib/core/`: chiede all'agente eventi e calendari e tiene
  lo stato in `ui-dati/gateway.db`) e i loghi (`GET` e `HEAD /logos/<file>`, più sotto); qualunque
  altro percorso che non sia una pagina è 404 in JSON. Il servizio Python `ui`, che prima serviva i loghi, non c'è più
  (l'ultimo commit che lo contiene è `142e789`). Del testo dei link ICS (copia in `ui-dati/ics_cache/`, rinnovata ogni 30 minuti) si
  occupa il gateway; le ricorrenze le espande l'agente (`POST /api/ics`, `server/agente-ts/src/ics.ts`, lo stesso
  codice che legge il telefono) e il gateway tiene in memoria l'ultima espansione riuscita di ogni link e
  intervallo: se l'agente non risponde e il gateway è appena ripartito, un link ICS mostra l'errore. Il
  compose dà al gateway `OE_DATI=/dati` (la cartella dei dati), `GATEWAY_DB=/dati/gateway.db`,
  `AGENTE_URL=http://agent:8180` e `AGENTE_CHIAVE` (la `API_KEY`, dai secret della Action o da `.env`, più sotto). **Lo stato** (calendari
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
  sessione. Il proprietario nasce a ogni avvio da `ADMIN_EMAIL` e `ADMIN_PASSWORD` (dai secret della Action o da `.env`, più sotto); le
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
# facoltativo: personalizzazione.json si fa dal modello personalizzazione.esempio.json, che sta nella radice del repository e non in server/
sudo install -o 10001 -g 10001 -m 600 personalizzazione.json /opt/oe-copilot/ui-dati/
# segreto dei cookie di sessione
openssl rand -hex 32
```
In `.env` (modello in `.env.esempio`): `UI_HOST=<nome del sito>`. Il prossimo aggiornamento costruisce e avvia
`gateway` e `caddy-ui` (senza la Action, a mano: `sudo docker compose up -d --build gateway caddy-ui`, con i secret come più sotto).

**Email e password del proprietario** (`ADMIN_EMAIL`, `ADMIN_PASSWORD`, almeno 12 caratteri, senza `$`), **chiave
dell'API** (`API_KEY`) e **segreto dei cookie** (`BETTER_AUTH_SECRET`, almeno 32 caratteri, `openssl rand -hex 32`)
stanno nei secret della Action e non in `.env`: l'aggiornamento (4.6) li passa allo script sullo standard input di SSH,
e compose li legge dall'ambiente prima che da `.env`. Per cambiarli:
`gh secret set ADMIN_PASSWORD` e un nuovo aggiornamento (anche `Run workflow` dalla pagina della Action):
il gateway riparte, la password nuova chiude le sessioni aperte. Con una email nuova nasce un utente nuovo e
quello vecchio resta con la sua password: va tolto da `gateway.db` (tabelle `session`, `account`, `user`).
Se i secret mancano vale ancora `.env`: è il caso di una prima installazione a mano o di un server senza la Action, dove `API_KEY`, `ADMIN_EMAIL`, `ADMIN_PASSWORD` e `BETTER_AUTH_SECRET` devono stare in `.env` o nell'ambiente della shell che lancia `docker compose` (con `sudo` serve `sudo -E`).
Su un server con la Action niente si ricrea a mano: i secret non stanno in `.env`, e un `docker compose up` ricrea l'agente con `API_KEY` vuota (non parte) e il gateway senza `BETTER_AUTH_SECRET` (si ferma con "BETTER_AUTH_SECRET deve essere impostata"). Dopo aver cambiato un valore di `.env` (`CALENDARIO_GID`, `UI_HOST`, `AUTH_2FA`, quelli di 4.9) si lancia un nuovo aggiornamento, `gh workflow run Server --repo danfortu/OE-Copilot --ref main` o *Run workflow* dalla pagina della Action: passa i secret e ricrea i servizi la cui configurazione è cambiata.

**HTTPS.** Caddy ottiene da solo il certificato, e per farlo il suo nome deve arrivargli da
internet. Dove la 443 è libera basta aggiungere a `caddy-ui`, in `docker-compose.yml`, le porte `443:443` e
`80:80`, senza togliere `127.0.0.1:4445:443`: il controllo del Caddy dell'aggiornamento (4.6) usa quella
porta, e senza quella porta la Action diventerebbe rossa a ogni aggiornamento. Sulla VM della guida la 443 è di sslh (SSH e HTTPS sulla stessa porta): si
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

#### Secondo fattore dell'accesso (facoltativo)

La variabile `AUTH_2FA` di `.env` (modello in `.env.esempio`), letta a ogni avvio del gateway, decide cosa chiede l'accesso dopo la password:
- `off` (predefinito, vale anche se la variabile manca o è vuota): non cambia niente. Email e password come sempre; le strade del secondo fattore (`/api/auth/two-factor/*`, `/api/auth/passkey/*`) e la pagina `/sicurezza` rispondono 404.
- `totp`: chi ha attivato il codice, dopo la password deve scrivere il codice di 6 cifre di un'app di autenticazione (Google Authenticator, Microsoft Authenticator, Aegis, 1Password e simili) oppure un codice di recupero. Chi non l'ha attivato entra come prima.
- `passkey`: tutto quello di `totp`, più le passkey. Nella pagina di accesso compare *Accedi con una passkey* (basta il dito, il volto o il PIN del dispositivo, niente password); la password resta come alternativa e, per chi ha il codice attivo, passa sempre dal codice. Una passkey è già un accesso completo e non chiede il codice. Il nome del sito e l'origine della passkey vengono da `APP_URL` (`https://<UI_HOST>`): se l'indirizzo del sito cambia, le passkey vecchie non valgono più e ne serve una nuova (nel frattempo basta la password).
- Un altro valore (un refuso) ferma l'avvio del gateway con `AUTH_2FA non valido` nel log (`docker logs oe-copilot-gateway`).
Sul server in uso c'è `AUTH_2FA=totp` dal 12/10/2026: finché il proprietario non attiva il TOTP dalla pagina `/sicurezza` (punto 1 qui sotto) l'accesso resta con la sola password.

Le tabelle del secondo fattore (`twoFactor`, `passkey` e la colonna `user.twoFactorEnabled` in `ui-dati/gateway.db`) il gateway le aggiunge a ogni avvio, anche con `off`: si aggiunge e non si toglie niente, e l'immagine di prima continua a funzionare sullo stesso database (non conosce il secondo fattore e lascia entrare con la sola password). Il segreto del codice sta cifrato con `BETTER_AUTH_SECRET`: cambiare quel segreto rende inutilizzabili i codici già attivati (punto 3 del recupero).

**Attivare il codice** (lo fa il proprietario, 3 minuti):
1. Sul server, in `/opt/oe-copilot/.env`, `AUTH_2FA=totp` (o `passkey`), poi un nuovo aggiornamento, `gh workflow run Server --repo danfortu/OE-Copilot --ref main` (o *Run workflow* dalla pagina della Action), che ricrea il gateway con il valore nuovo. Un `docker restart` non basta: l'ambiente di un container si fissa quando lo si crea. Il deploy non tocca `.env`. Senza la Action: `cd /opt/oe-copilot && sudo docker compose up -d gateway`, con i secret in `.env` o nell'ambiente (4.7, sopra).
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
   # poi, dal PC: gh workflow run Server --repo danfortu/OE-Copilot --ref main   (senza la Action: sudo docker compose up -d gateway, con i secret in .env o nell'ambiente)
   docker logs --tail 5 oe-copilot-gateway    # a deploy finito deve dire: secondo fattore dell'accesso (AUTH_2FA): off
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

#### Gruppi di calendari

Nella finestra *Calendari da mostrare* (*Gestisci calendari*, a sinistra) si creano, rinominano ed eliminano i gruppi, e i pulsanti accanto al nome di ogni calendario lo mettono dentro o fuori da un gruppo: un calendario può stare in più gruppi. Nella barra a sinistra un pulsante per gruppo lascia accesi solo i suoi calendari (un secondo clic li riaccende tutti). Stanno nella configurazione, voce `gruppi_calendari`, per nome come li mostra la barra: `{"gruppo1": ["Alfa", "Beta"], "gruppo2": ["Gamma"]}`; se si rinomina l'azienda dalla finestra, i gruppi seguono il nuovo nome.

#### Account uniti e calendari secondari

Nella finestra *Calendari da mostrare* (*Gestisci calendari*) la sezione *Unisci account sotto un nome* mette due o più account sotto un nome solo: si spuntano gli account, si scrive il nome (per esempio *Personale*) e *Unisci*. Nella barra a sinistra sono una voce sola, con un nome, un logo, un interruttore e un pallino (vale il peggiore dei loro stati); nella finestra stanno in un blocco solo, con gli indirizzi sotto l'intestazione e *Separa* accanto a ognuno (l'account torna da solo, col suo indirizzo come nome, che si cambia subito dopo). Cambiare il nome nell'intestazione lo cambia per tutti gli account del blocco. Non c'è niente di nuovo da salvare: "uniti" vuol dire che hanno lo stesso nome mostrato (voce `account_aliases` della configurazione, che vince su `nomi_per_indirizzo` e `nomi_per_dominio` di `personalizzazione.json`), e il nome di un'azienda nelle voci di `fonti.json` non conta per la barra. Il nome di un account unito che compare in un gruppo di calendari resta una volta sola.

Il pulsante *Secondario* accanto al nome di ogni voce la mette sotto *Altri calendari*, un gruppo chiuso in fondo alla barra (si apre da solo con una ricerca), e rende più tenui i suoi eventi nella griglia. È la voce `secondari_scelti` della configurazione (nome -> vero o falso); sopra c'è `interfaccia.calendari_secondari` di `personalizzazione.json`, che vale finché la finestra non dice altro per quel nome (falso la toglie anche se il file la nomina). Un cambio di nome porta con sé la scelta, come i gruppi e il logo.

La colonna di sinistra (minicalendario e calendari) si allarga e si stringe trascinando il suo bordo, da 220 a 480 px; le frecce sinistra e destra la muovono di 16 px e un doppio clic la riporta a 256. Trascinando, la colonna segue il puntatore a ogni fotogramma e anche gli eventi si rimisurano subito: FullCalendar rimisura la griglia solo su un evento `resize` della finestra e di norma aspetta 100 ms dall'ultimo (`windowResizeDelay`), perciò `Calendario.tsx` lo porta a 0 e la colonna manda un evento `resize` a ogni passo. Misurato in un browser vero su un giorno con 24 riunioni: 60 fotogrammi al secondo, la colonna a meno di mezzo pixel dal puntatore, circa 0,3 ms di ricalcolo del browser e 2 ms di rimisura per passo. Stato, archivio e l'ultimo evento arrivano al rilascio. La larghezza resta in questo browser (`localStorage`, chiave `oe.larghezza-pannello`) e non sul server, quindi ogni PC o telefono ha la sua. Una voce con più account uniti ha sotto il nome "N account uniti".

#### Disponibilità pubblica

`https://<UI_HOST>/disponibilita` si apre senza accesso: è lo stesso calendario (viste, minicalendario, zoom, tema) con ogni impegno ridotto a un blocco "Occupato", i soli orari. Impegni che si sovrappongono o si toccano sono un blocco solo; le festività restano. Non ci sono titoli, luoghi, partecipanti, nomi di calendari o di aziende, né ricerca, nuovo evento, legenda, timesheet e impostazioni. `?gruppo=gruppo1` conta solo i calendari di quel gruppo. Contano tutti i calendari, anche quelli spenti nella barra laterale (gli interruttori sono un filtro della vista), e i giorni interi non occupano; se una sorgente non risponde la pagina lo scrive. I dati: `GET /api/disponibilita?da=AAAA-MM-GG&a=AAAA-MM-GG[&gruppo=nome]` (al più 62 giorni), nella forma di `get_events` con solo "Occupato". Il gateway li legge a blocchi di 28 giorni, li tiene un minuto e fa al più 20 letture al minuto in tutto (dietro sslh i client non si distinguono); ogni blocco lo legge il metodo `get_disponibilita` (solo orari, `server/web-ts/src/lib/core/`).

#### Loghi delle aziende

Accanto al nome di ogni azienda nella barra a sinistra, e sugli eventi del calendario (nell'angolo, e nelle righe della vista mese e agenda), compare il suo logo; senza logo restano le iniziali nel colore dell'azienda. Si dichiarano in `ui-dati/personalizzazione.json`, voce `interfaccia.loghi`: il nome come lo mostra la barra e il file (o un indirizzo intero, più sotto), per esempio `{"interfaccia": {"loghi": {"Alfa": "logos/alfa.png"}}}`. I file stanno sul server in `ui-dati/logos/` (PNG quadrati, 128 px bastano; la cartella non è nel repository, come il resto dei dati di chi usa l'app) e il gateway li serve a `GET /logos/<file>` (anche `HEAD`) dopo l'accesso. Se il file non c'è lì, si cerca tra quelli che porta l'immagine del gateway (`server/web-ts/logos-predefiniti/`, in `/app/logos-predefiniti` nel container; la cartella si cambia con `LOGHI_PREDEFINITI`): oggi solo `personale.png`, il logo degli *Eventi personali*, che la pagina chiede da sé senza dichiararlo in `personalizzazione.json`. A parità di nome vince `ui-dati/logos/`: un `personale.png` messo lì sostituisce quello dell'immagine. Il tipo viene dall'estensione (png, jpg, jpeg, gif, webp, svg, ico; qualunque altra è `application/octet-stream`). Un nome che esce dalla cartella (con `..`, assoluto, o un collegamento che porta fuori), una cartella o un file che non c'è risponde 404. Non si riavvia niente: un file nuovo si serve subito, e `personalizzazione.json` il gateway lo rilegge quando cambia. Per aggiungere un'azienda: copiare il file in `ui-dati/logos/` (leggibile dall'utente 10001, come il resto di `ui-dati/`) e aggiungere la riga, oppure scegliere il logo dalla finestra *Gestisci calendari* (più sotto). La pagina pubblica `/disponibilita` non mostra né nomi né loghi.

**Loghi da un indirizzo e dalla finestra Calendari.** Il valore di una voce di `interfaccia.loghi` può essere anche un indirizzo intero, per esempio `{"Beta": "https://beta.example/logo.png"}`. Prima lo apriva il browser di chi guarda la pagina, dal sito del logo (né la pagina né Caddy lo impediscono): il sito vedeva l'indirizzo IP di chi apre il calendario, e il logo spariva quando il sito lo toglieva. Ora lo scarica il gateway, una volta sola, e lo tiene in `ui-dati/logos/url-<16 cifre di sha256 dell'indirizzo>.<estensione>` (l'estensione la decidono i primi byte del file, non il nome): il browser chiede sempre e solo `/logos/<file>` a questo sito. Per un indirizzo scritto a mano in `personalizzazione.json` lo scarico parte in secondo piano la prima volta che la pagina chiede la mappa e si rinnova ogni 7 giorni: finché il file non c'è la barra mostra le iniziali e la pagina rilegge la mappa ogni 6 secondi, al più 5 volte; se lo scarico non riesce si riprova dopo 5 minuti e intanto resta il file di prima. Il valore si legge come lo legge il browser: `/\beta.example/logo.png`, `\\beta.example\logo.png` e un indirizzo spezzato da un tab o da un a capo sono lo stesso indirizzo di `//beta.example/logo.png` (https) e il gateway li scarica come gli altri. La pagina non carica mai un logo da un altro sito: un valore che porta fuori, o che ha uno schema che il gateway non scarica (`ftp:`, `data:`), non si mostra e restano le iniziali. I file con il solo nome (`logos/alfa.png`) funzionano come sempre, e un file dichiarato che non c'è lascia le iniziali, mai l'immagine rotta.

Nella finestra *Calendari da mostrare* (*Gestisci calendari*) ogni voce della barra ha accanto al nome il pulsante *Logo*: le caselle (con il nome che hanno), *Eventi personali* (predefinito: `personale.png`), *Festività* e i calendari da link. Il pannello ha il campo dell'indirizzo, *Anteprima* (il server scarica e mostra l'immagine, senza salvare niente), *Usa questo logo*, *Rimuovi* (si torna al logo di `personalizzazione.json`, o alle iniziali), *Aggiorna* (scarica di nuovo l'indirizzo) e *Carica un file* (stessi controlli e stessi limiti). Scrivendo solo un dominio (`esempio.org`) la finestra offre l'icona del sito, `https://esempio.org/favicon.ico`. Le scelte stanno nella tabella `loghi` di `ui-dati/gateway.db` (colonne `nome`, `url`, `file`, `tipo`, `aggiornato`; si aggiunge da sola all'avvio): non sono nella configurazione, quindi `config.json` e `get_config` non cambiano, e vincono su `personalizzazione.json`. Se un'azienda cambia nome dalla finestra (alias) il logo la segue, come i gruppi: se due caselle hanno lo stesso nome e se ne rinomina una, il logo va con il nome nuovo e l'altra torna alle iniziali. La cartella non cresce: al più 60 scelte e 40 indirizzi di `personalizzazione.json`, quindi al più 100 file `url-*` e `up-*`, e il file di una scelta sostituita o tolta si cancella; i file messi a mano non si toccano (la pulizia guarda solo i nomi `url-` o `up-` seguiti da 16 cifre esadecimali e da un'estensione di immagine).

**Che cosa scarica il gateway.** L'indirizzo lo apre solo il gateway, dopo l'accesso (metodi `get_loghi`, `preview_logo`, `set_logo`, `upload_logo`, `refresh_logo` e `remove_logo`, con le regole di ogni metodo: sessione, stesso sito, JSON), e solo se porta a un sito pubblico: `http` o `https` sulle porte 80 e 443, senza nome utente e password nell'indirizzo, e il nome deve risolversi solo a indirizzi pubblici (si rifiutano loopback, reti private, link-local compreso 169.254.169.254 dei metadati del cloud, CGNAT 100.64.0.0/10, IPv6 unique-local, link-local e site-local, IPv4 mappati e tradotti in IPv6, NAT64, multicast e riservati; basta un indirizzo che non va per rifiutare il nome). Un nome che porta alla rete interna risponde come un nome che non esiste (*L'indirizzo non risponde*), così dal testo non si ricava quali nomi interni esistono; un indirizzo scritto con l'IP interno è *Indirizzo non ammesso*. Il controllo ricomincia a ogni reindirizzamento (al più 3) e si ripete sull'indirizzo a cui ci si collega davvero. Il tempo totale è di 8 secondi, si leggono al più 1 MB, in un buffer solo e in al più 20000 pezzi (un corpo a pezzi da un byte costa un evento a pezzo: un milione fermava il gateway per mezzo secondo e ne alzava la memoria di 400 MB), e se ne accettano 512 KB. I collegamenti verso l'esterno stanno aperti al più 4 insieme, tra i metodi della finestra e lo scarico in secondo piano (che ne usa al più 3: l'ultimo resta a chi preme un pulsante); 16 richieste aspettano il loro turno per al più 5 secondi, le altre ricevono *Troppe richieste, riprova tra poco*. Il tipo lo decidono i primi byte (PNG, JPEG, GIF, WebP, ICO, SVG), mai il Content-Type del sito, e di un'immagine si legge la grandezza dalla testata: oltre 4096 pixel per lato o 16 megapixel si rifiuta (anche per l'immagine dentro un ICO; di un GIF si guardano ogni fotogramma e la tela, che cresce quanto serve ai fotogrammi, non solo lo schermo logico). Uno SVG si legge una volta sola da sinistra a destra, in tempo lineare, come lo divide un analizzatore XML: deve dividersi per intero (UTF-8, al più 256 livelli, un solo DOCTYPE pubblico di W3C e senza sottoinsieme interno) e si rifiuta con script, `foreignObject`, gestori di eventi, `javascript:`, `data:`, riferimenti che non siano `#frammenti` (anche se un'animazione li scrive) o CSS che carica da fuori (anche in un attributo come `fill`, `filter` o `cursor`). Ogni risposta di `/logos/` porta `X-Content-Type-Options: nosniff` e `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`: un logo aperto da solo in una scheda non esegue niente. Gli errori che la finestra mostra: *Indirizzo non ammesso*, *L'indirizzo non risponde*, *Non è un'immagine*, *Immagine troppo grande (massimo 512 KB)*, *Immagine troppo grande (massimo 4096 pixel per lato e 16 megapixel)*, *SVG non ammesso: contiene codice o riferimenti esterni*, *Troppe richieste, riprova tra poco*, *Non riesco a salvare il logo sul server* (la cartella `ui-dati/logos` non è scrivibile dall'utente 10001: sezione 7, *Diagnosi rapida*). Il codice sta in `server/web-ts/src/lib/core/` (`rete-sicura.ts`, `immagine.ts`, `semaforo.ts`, `loghi.ts`) e in `server/web-ts/src/lib/valore-logo.ts`, senza dipendenze nuove; le prove sono nella sezione 7.

#### I calendari da link e la guardia di rete

`add_ics_calendar` e il rinnovo ogni 30 minuti (`ics.ts`) scaricano con `rete-sicura.ts` e con le stesse regole sugli indirizzi dei loghi: un calendario su un indirizzo privato (rete interna, VPN, rete di Docker, 127.0.0.1) non si può più sottoscrivere, e il link di un altro OE Copilot (4.10) vale solo se porta a un sito pubblico; un calendario già sottoscritto così tiene l'ultima copia e mostra l'avviso *Indirizzo non ammesso*. Cambiano i limiti: 15 secondi in tutto, al più 8 MiB letti e accettati, nessun Content-Type richiesto (se c'è: `text/calendar`, `text/plain` o `application/octet-stream`) e il testo deve cominciare con `BEGIN:VCALENDAR` dopo un eventuale BOM e degli spazi; una risposta che non è un calendario non sostituisce la copia. Due limiti noti: un server che dà il calendario con un altro tipo (per esempio `application/ics`) è rifiutato, e un calendario vicino agli 8 MiB passa dal gateway ma può superare gli 8.000.000 byte che l'agente accetta nel corpo di `POST /api/ics` (il testo ci arriva in JSON e ogni fine riga raddoppia di grandezza): allora dà errore all'agente. Restano la copia di 30 minuti, la copia vecchia mostrata quando il rinnovo non riesce, 404 e 410 che la tolgono (le festività no) e il testo `Il link non risponde: ...`, il cui indirizzo è quello dell'ultima tappa e quindi pubblico. I posti sono quelli dei loghi (4 insieme, 16 in coda per 5 secondi). Il punto di prova e i posti stanno in `server/web-ts/src/lib/core/uscita.ts`.

#### Personalizzazione

Senza `personalizzazione.json` in `ui-dati/` le aziende prendono il nome dal
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
5. Il compose passa i due valori al gateway, che li legge solo all'avvio: un nuovo aggiornamento lo ricrea, `gh workflow run Server --repo danfortu/OE-Copilot --ref main` (o *Run workflow* dalla pagina della Action; senza la Action `cd /opt/oe-copilot && sudo docker compose up -d gateway`, con i secret in `.env` o nell'ambiente, 4.7). Un `docker restart` non basta, perché non rilegge `.env`.
6. Nell'app: *Impostazioni*, sezione *Promemoria*. Sotto *Dove* la riga di Telegram non deve più dire "non configurato sul server". *Invia una prova* manda il testo fisso "Prova dei promemoria", mai dati di una riunione: se arriva nella chat il canale funziona. Poi si accende *Avvisami prima delle riunioni*. Senza un canale configurato l'interruttore non si accende.

**Notifiche del browser (il secondo canale, facoltativo).** Il server spedisce il messaggio al servizio di notifiche del browser (Google, Mozilla o Apple, secondo il browser), che lo consegna al browser a pagina chiusa. Il testo viaggia cifrato da un capo all'altro (RFC 8291): il servizio di notifiche vede a chi e quando, non cosa. Non funziona dentro le finestre dell'app Tauri (Windows e Android), che non hanno le notifiche web (misurato il 12/10/2026): per quelle resta Telegram. Il client in `app/` avvisa comunque da sé delle riunioni in arrivo, con le notifiche di Windows o di Android, ma solo finché è in esecuzione (`app/README.md`, sezione Notifiche). Serve una coppia di chiavi VAPID, una volta sola, che si fa con node (senza installare niente):
   ```bash
   node -e "const c=require('crypto');const j=c.generateKeyPairSync('ec',{namedCurve:'P-256'}).privateKey.export({format:'jwk'});console.log('VAPID_PUBLIC_KEY='+Buffer.concat([Buffer.from([4]),Buffer.from(j.x,'base64url'),Buffer.from(j.y,'base64url')]).toString('base64url'));console.log('VAPID_PRIVATE_KEY='+j.d)"
   ```
   Le due righe che escono vanno in `.env` (la privata è un segreto come il token: lockbox, mai in chat), con `VAPID_SUBJECT=mailto:<indirizzo>` (o `https://<sito>`), il contatto che il servizio di notifiche può usare se il server si comporta male. Poi si ricrea il gateway come al punto 5. Nelle Impostazioni, sotto *Dove*, *Notifiche del browser* offre *Attiva su questo browser*: il browser chiede il permesso e iscrive sé stesso (si fa una volta per browser o telefono, fino a 20). Poi *Invia una prova* e l'interruttore. *Togli da questo browser* toglie l'iscrizione, e il server toglie da solo quelle che il servizio dichiara scadute. L'indirizzo di un'iscrizione vale come una password: sta solo in `ui-dati/gateway.db` (tabella `push_iscrizioni`), non compare nei log né nelle risposte dell'app, e il server accetta solo indirizzi https con un nome di servizio (mai un indirizzo IP o un nome della rete interna) e non segue i reindirizzamenti. Il messaggio è lo stesso di Telegram: la prima riga è il titolo della notifica, il resto il corpo.

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

**Spegnerlo.** L'interruttore nelle Impostazioni vale dal giro dopo, senza riavvio. Per toglierlo del tutto: via i valori dei canali da `.env` (`TELEGRAM_*`, `VAPID_*`) e si ricrea il gateway come al punto 5 di *Attivarlo*.

**Prove.** `cd server/web-ts && npx vitest run test/promemoria`: motore, canale Telegram contro un server HTTP locale, notifiche del browser contro l'esempio dell'RFC 8291 e un servizio finto, e metodi dell'API attraverso il gateway, senza rete.

### 4.10 Condividere il calendario con un altro OE Copilot (facoltativo)
Due istanze di OE Copilot (la tua e quella di un amico, su un'altra VM) possono mostrarsi il calendario a vicenda: la tua pubblica un link ICS per ogni persona, l'altra lo importa con la sottoscrizione di calendari da link che ha già (*Gestisci calendari → Aggiungi un calendario da link*), senza niente di nuovo dalla sua parte, e lo rilegge da sola ogni 30 minuti. Non è l'importazione in una casella di posta: nessun account viene toccato, il calendario resta uno in più dentro l'altro OE Copilot.

**Il link.** Ogni persona ha il suo: nome di chi lo riceve, livello, cosa condividere, giorni indietro e avanti, nome del calendario, ora dell'ultima lettura e numero di letture. L'indirizzo è `https://<UI_HOST>/condividi/<testo segreto>.ics`; il testo segreto sono 32 byte casuali in base64url (43 caratteri), uno per link. Il percorso è pubblico (niente accesso, niente cookie) e risponde `text/calendar; charset=utf-8` con `Cache-Control: private, max-age=300`, `ETag` e `Last-Modified` (304 per `If-None-Match` e `If-Modified-Since`), `X-Robots-Tag: noindex, nofollow`, `X-Content-Type-Options: nosniff` e il nome di file `calendario.ics`. Il calendario dichiara `X-PUBLISHED-TTL:PT30M` e `REFRESH-INTERVAL;VALUE=DURATION:PT30M` (la rilettura ogni 30 minuti, la stessa durata della copia che un OE Copilot tiene dei link ICS) e `X-OE-COPILOT-SHARE:1`, che insieme al `PRODID` `-//OE Copilot//Condivisione//IT` è il segno con cui un altro OE Copilot riconosce un calendario condiviso (tutte e due le righe sono scritte nel codice: un calendario ostile potrebbe scriverle, e l'unico effetto sarebbe che i suoi eventi non rendono occupata la pagina pubblica di chi lo ha importato).

**Creare, cambiare, revocare.** *Impostazioni → Condivisione → Gestisci i link di condivisione*: l'elenco (nome, livello, cosa si condivide, ultima lettura come "12 min fa" o "mai", numero di letture, indirizzo con il pulsante *Copia*), *Modifica*, *Cambia indirizzo* e *Revoca* (con una conferma) e il modulo del link nuovo; sotto la scelta del livello c'è l'avviso di cosa vede chi ha il link. I link stanno nella tabella `condivisioni` di `ui-dati/gateway.db`, che il gateway crea all'avvio (si aggiunge, non tocca le altre); non stanno nella configurazione, quindi `config.json` e la sua copia non li hanno. I metodi dell'API (con la sessione, `POST /api/<metodo>`, le stesse regole degli altri): `get_condivisioni`, `add_condivisione`, `update_condivisione`, `revoke_condivisione`, `delete_condivisione`, `rotate_condivisione`; gli errori sono frasi in italiano (nome mancante o troppo lungo, livello o gruppo che non c'è, giorni fuori dai limiti, più di 50 link) e non contengono mai il testo segreto. L'ultima lettura e il conto si scrivono nel database al più una volta al minuto per link.

**Cosa esce.** Due livelli, uno per link. *Occupato* (di norma): gli stessi blocchi della disponibilità pubblica (4.7), cioè gli impegni uniti (quelli che si sovrappongono o si toccano sono un blocco solo), ognuno un evento `SUMMARY:Occupato` con `DTSTART` e `DTEND` in UTC (`...Z`), `TRANSP:OPAQUE`, `STATUS:CONFIRMED` e niente altro: né titolo, luogo, organizzatore, partecipanti, descrizione, link per collegarsi, nome di azienda o di calendario, né l'identificativo di partenza. *Con titoli*: un evento per impegno, con titolo e luogo; mai descrizione, partecipanti, organizzatore, link per collegarsi (anche quelli dentro il titolo o il luogo, con o senza `https://`: `bit.ly/X`, `8x8.vc/X/sala`, `app.slack.com/huddle/X`, un indirizzo incollato a un'etichetta come `Meet:meet.google.com/x`; un indirizzo esce tutto, fino allo spazio, anche con parentesi e virgole dentro), il luogo che dice solo "Microsoft Teams Meeting" e il luogo che parla di ID della riunione, codice di accesso, PIN o numero da chiamare (chi ha il link alla riunione ha anche quelli), indirizzi di posta nel luogo, nomi di aziende o di caselle, caratteri invisibili e di direzione (spazi a larghezza zero, U+202E e gli altri: si tolgono dal titolo e dal luogo). Titolo e luogo si guardano per i primi 1000 caratteri e si tagliano a 250. Gli indirizzi di posta si tolgono dal luogo e non dal titolo: un indirizzo scritto nel titolo esce com'è, a chi ha un link con i titoli. L'identificativo (`UID`) di un evento è stabile per lo stesso evento nello stesso link e non dice niente: un HMAC dell'orario (e, con i titoli, del testo) con un sale casuale del link, che sta nel database e non si pubblica. I giorni interi: nel livello occupato non contano (come nella disponibilità pubblica); con i titoli escono come giorni interi se la pagina li mostra (la voce `mostra_tutto_il_giorno` di `personalizzazione.json`, descritta nel README, per le caselle; quelli degli eventi personali escono sempre). Il cambio d'ora di ottobre: nella notte dell'ultima domenica l'ora fra le 2 e le 3 c'è due volte e le caselle la danno senza dire quale; un evento in quell'ora esce con la prima (come un `datetime` di Python con `fold=0`), quindi uno della seconda può uscire un'ora prima. È lo stesso limite della pagina pubblica della disponibilità (4.7), che mostra l'ora scritta senza dire quale delle due.

**Cosa si condivide.** *Tutti i calendari accesi*: valgono gli interruttori della barra laterale, un'azienda spenta non esce (esce quello che si vede nel calendario). Oppure *un gruppo* (4.7): contano le aziende del gruppo e basta, come in `/disponibilita?gruppo=`; gli interruttori della barra laterale non c'entrano (un'azienda del gruppo spenta nella barra esce lo stesso, e la finestra di condivisione lo dice sotto la scelta del gruppo). I calendari che l'app ha ricevuto da altri (link ICS, festività, calendari condivisi da un altro OE Copilot) non escono mai, in nessun livello: due istanze che si importano a vicenda non si rimandano in giro i calendari. Gli eventi personali escono come gli altri. Finestra: da 0 a 30 giorni indietro e da 1 a 365 avanti da oggi (di norma 7 e 90). Il gateway legge a blocchi di 28 giorni come per la disponibilità e tiene il testo di ogni link un minuto, così un lettore insistente non carica l'agente: un cambio agli interruttori, ai gruppi, ai nomi delle aziende o agli eventi arriva nel link entro un minuto, e a chi lo legge entro altri 30 minuti; revocare o cambiare indirizzo vale subito (il testo segreto si controlla a ogni richiesta, il minuto è solo del calendario).

**Come lo importa l'altro OE Copilot.** In *Gestisci calendari*, in fondo, *Aggiungi un calendario da link*: incolla il link, il nome può restare vuoto (prende quello che hai pubblicato, di norma "Calendario condiviso"), *Aggiungi e verifica*. Il calendario compare nella sua barra con i blocchi "Occupato" (o i titoli) e il suo OE Copilot lo rilegge al più ogni 30 minuti (la copia dei link ICS, `ics_cache/`, dura 30 minuti). Va bene anche una versione più vecchia dell'app, perché usa soltanto la sottoscrizione ICS che c'era già. Da questa versione i calendari condivisi da un altro OE Copilot non rendono occupata la pagina pubblica di chi li ha importati (i loro eventi portano il segno `X-OE-COPILOT-SHARE:1` con il `PRODID` di OE Copilot, e `get_disponibilita` li lascia fuori); una versione più vecchia li conterebbe come impegni suoi.

**Revocare e cambiare indirizzo.** *Revoca*: l'indirizzo dà 404 subito, il testo segreto si cancella dal database e la riga resta (per vedere chi era e quando ha letto per ultimo) finché non si toglie con *Togli dall'elenco*. *Cambia indirizzo*: ne nasce uno nuovo, il vecchio dà 404 subito e il nuovo va mandato a chi legge, che lo sostituisce a mano nel suo OE Copilot. Chi ha importato il link smette di vedere i dati alla prima lettura dopo la scadenza della copia, al più 30 minuti dopo: un OE Copilot aggiornato, davanti a un 404 o a un 410, toglie la copia dal disco, non mostra più gli eventi e segna il calendario in rosso ("il link non risponde più (404): è stato revocato o cambiato"); resta così, senza eventi, finché una lettura non riesce di nuovo (un link che torna a rispondere si legge da capo); uno più vecchio tiene l'ultima copia con un avviso giallo finché il calendario non si toglie. Vale per 404 e 410: per un guasto (500, 503, rete caduta) ogni versione tiene l'ultima copia ("meglio dati vecchi che nessun dato").

**Un link è una chiave.** Chi ha l'indirizzo legge il calendario, senza accesso e senza altro: va dato solo a chi deve vederlo, mai in una chat di gruppo o in un ticket. Un indirizzo che non vale (non c'è, è revocato o cambiato, è malformato) riceve sempre lo stesso 404, e il confronto del testo segreto è in tempo costante (SHA-256 di lunghezza fissa e `timingSafeEqual`, su tutti i link). Chi sbaglia più di 20 indirizzi in un minuto dallo stesso indirizzo di rete riceve 429 per il resto del minuto; dietro Caddy e sslh l'indirizzo di rete è quello che Caddy ha visto (l'ultimo valore di `X-Forwarded-For`, che scrive Caddy: i precedenti li scrive chi chiede e non contano), quindi con sslh è uno solo per tutti. Un indirizzo che vale non si ferma mai per quel conto. Il gateway non scrive il testo segreto in nessun log né in nessun errore (solo l'id del link, 12 cifre). Il testo segreto però sta in chiaro nella tabella `condivisioni` di `ui-dati/gateway.db`, perché il pulsante *Copia* deve poterlo dare di nuovo: chi legge quel file, o una copia di sicurezza di `ui-dati/`, ha tutti i link attivi, e la copia di sicurezza va trattata come i dati del telefono. Un link revocato perde il suo testo.

**Log.** Caddy (`server/caddy-ui/Caddyfile`) non scrive un log di accesso, ma quando non riesce a passare una richiesta al gateway (gateway fermo o in riavvio, per esempio durante un deploy: 502) scrive l'errore con la richiesta intera, `uri` e intestazioni comprese: verificato con il Caddy 2.11.4 vero, con il gateway giù il log aveva `"uri":"/condividi/<testo segreto>.ics"`, e il testo segreto c'era anche con `//condividi/...`, con l'indirizzo intero di una richiesta fatta a un proxy, con `/x/../condividi/...`, con `/CONDIVIDI/...` e nell'intestazione `Referer`. Per questo il blocco globale del Caddyfile ha un filtro (`log { format filter { ... } }`) che nell'`uri` lascia `/condividi/REDATTO`: tutto quello che segue `/condividi/` fino al punto interrogativo, maiuscole comprese e comunque sia scritto il percorso, e dovunque sia ogni sequenza di 32 o più caratteri base64url (com'è fatto il testo segreto); le intestazioni della richiesta (`request>headers`) non si scrivono. Senza il filtro il testo segreto starebbe in `docker logs oe-copilot-caddy`. Con il Caddy vero e un gateway che non c'è, 17 forme dell'indirizzo e delle intestazioni non lasciano niente nel log. sslh vede solo il nome del sito (SNI): il percorso viaggia cifrato. Il testo segreto resta comunque nella cronologia del browser e nelle app di chi ha il link, e nei log dell'altro OE Copilot se la sua lettura fallisce per un motivo diverso da 404 (il testo dell'errore ha l'indirizzo, come per ogni calendario da link).

**Quando il server non legge.** Se l'agente non risponde il link dà 503 con `Retry-After`, mai un calendario vuoto: per chi lo importa un calendario vuoto vorrebbe dire "libero". L'altro OE Copilot tiene l'ultima copia e segna un avviso giallo. Lo stesso vale se il gruppo scelto da un link non c'è più (la pagina lo scrive sul link), e se l'agente risponde ma una casella o un'azienda che il link pubblica no: il servizio dà 200 con lo stato `errore` della casella (`calendar.db` non leggibile, accesso scaduto, aggiornamento sul server fermo da più di 15 minuti), la pagina la mostra con il pallino rosso e il link dà 503 al posto di un calendario a cui mancano i suoi eventi. Con *tutti* contano le aziende accese, con un gruppo quelle del gruppo (comprese le aziende inoltrate, che si leggono dalla casella che ne ospita le copie, e gli eventi personali quando non si leggono); un'azienda fuori da quello che il link pubblica può essere in errore senza fermarlo. Gli stati di avviso (telefono bloccato, aggiornamento in ritardo o in pausa, nessun segno di vita) non fermano niente: i dati ci sono. Stesso 503 se gli eventi nella finestra sono più di 5000: pubblicare i primi darebbe per liberi i mesi dopo, e si accorciano i giorni del link. Chi legge il link vede sempre lo stesso testo; il motivo sta nel log del gateway (`condivisione <id>: calendario non disponibile, ...`).

**Prove.** Elenco, comandi e passi della Action: sezione 7 (*Prove della condivisione del calendario*).

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

3. **Chiamata all'agente**. La chiave dell'API sta nell'ambiente dell'agente sul server (con l'aggiornamento automatico arriva dai secret della Action e in `.env` non c'è; in una prima installazione a mano sta in `.env`): si legge in una variabile e non si stampa.
   ```bash
   CHIAVE=$(ssh oecopilot "docker exec oe-copilot-agent printenv API_KEY")
   curl -s -H "Authorization: Bearer $CHIAVE" http://127.0.0.1:13180/api/calendari
   ```

### L'agente (TypeScript)
`server/agente-ts/` è l'agente: legge in sola lettura il `calendar.db` del telefono e `agent/fonti.json` e risponde con un'API a chiave (`/api/eventi`, `/api/calendari`, `/api/salute`, `POST /api/ics`). È il servizio `agent` di compose (container `oe-copilot-agent`, porta `127.0.0.1:3180`, tunnel come sopra) e serve il gateway (`AGENTE_URL=http://agent:8180`, il valore di partenza del compose). Lo ricostruisce e controlla che risponda l'aggiornamento automatico (4.6); in una prima installazione a mano si avvia con `docker compose up -d agent` (4.3), con `API_KEY` in `.env` o nell'ambiente. Gira come utente 10001, a file system in sola lettura e senza capacità.

I file del calendario sono del gruppo 10019 (l'utente di Android che li scrive) e il gruppo entra con `group_add` nel file di compose. Se Android cambia il numero, `sudo stat -c %g /opt/oe-copilot/telefono/data/data/com.android.providers.calendar/databases/calendar.db` lo dice, in `.env` si scrive `CALENDARIO_GID=<numero>` (modello in `.env.esempio`) e un nuovo aggiornamento ricrea l'agente, `gh workflow run Server --repo danfortu/OE-Copilot --ref main` (o *Run workflow* dalla pagina della Action; senza la Action `sudo docker compose up -d agent`, con `API_KEY` in `.env` o nell'ambiente: con la chiave vuota l'agente non parte): senza il gruppo le caselle risultano in errore con "Permission denied" in `/api/eventi` e `/api/salute`.

L'agente in TypeScript ha sostituito quello in Python (sezione 2) dopo essere stato provato fianco a fianco: le risposte erano le stesse byte per byte (su un calendario inventato in CI, su una copia di quello del telefono e su quello vivo), con tre differenze volute che restano: un percorso con byte non ASCII non codificati (senza `%`) riceve 400, perché Node li rifiuta (il gateway codifica sempre); uno spazio dopo la chiave giusta in `Authorization` passa, perché Node lo toglie (il Python rispondeva 401); un testo ICS senza a capo non viene mai preso per il percorso di un file (il Python lo apriva se esisteva). Le risposte registrate del Python sono le prove dell'agente (sezione 7).

---

## 6. Il rilevatore degli accessi scaduti

Chiude il "falso verde" (vedi [ragionamenti](#1-i-ragionamenti-perché-è-fatto-così)).
Il rilevatore (`supervisore/src/rilevatore/`), lanciato dal supervisore ogni 2 giri (circa 70 secondi), scrive in `stato/accessi.json`
lo stato di ogni casella, che l'agente usa per il pallino, **senza mai leggere un token**:

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

Il rilevatore gira nel supervisore (4.5), in TypeScript (`supervisore/src/rilevatore/`): legge gli stessi file di Outlook (database e log, in sola lettura) e `dumpsys content` dal telefono con adb. `supervisore/test/differenziale.test.ts` lo confronta, a ogni modifica, con le uscite registrate del rilevatore in Python (`supervisore/test/golden/rilevatore-accessi.json`, registrate una sola volta su 268 casi inventati: log con la rotazione, database, battito, `dumpsys content`, ora): `accessi.json` e `google-sync.json` devono uscire uguali byte per byte. L'unica differenza voluta: con byte non UTF-8 in `dumpsys content` il Python non dava verdetti per Google e il TypeScript sì. Il Python (`strumenti/rileva_accessi.py`) è stato tolto dal repository e sta nella cronologia di git (sezione 2).

---

## 7. Manutenzione e diagnostica

- **Rifare un login scaduto**: dalla pagina `/telefono/` (4.8; finché è aperta la sincronizzazione sta in pausa), poi rifai il login nell'app Outlook o nelle impostazioni dell'account Google del telefono e chiudi la scheda.
- **Segni di vita** (sul server): `stato/battito.json` (il servizio è vivo? il telefono è acceso? Outlook è in primo piano?) e
  `stato/accessi.json` (stato di ogni casella).
- **Log dei servizi** (sul server): `docker logs oe-copilot-<servizio>` (`agent`, `gateway`, `schermo`, `caddy`, `supervisore`, `telefono`).
- **Vedere cosa fa l'agente**: `curl` su `http://127.0.0.1:3180/api/calendari` dal server (con la
  chiave, vedi 4.5), oppure dal PC col tunnel della sezione 5.
- **Aggiornare il codice del server**: con l'aggiornamento automatico (sezione 4.6) basta un push
  su `main`, oppure `gh workflow run Server --repo danfortu/OE-Copilot --ref main` per ripeterlo. Senza la Action, a mano: `git pull`, ricopia `server/` in `/opt/oe-copilot` (mantieni `.env`, `agent/fonti.json`, `telefono/`, `stato/` e `ui-dati/`) e, con `API_KEY`, `ADMIN_EMAIL`, `ADMIN_PASSWORD` e `BETTER_AUTH_SECRET` in `.env` o nell'ambiente (4.7), `sudo docker compose up -d --build --remove-orphans` (toglie anche i container dei servizi che non ci sono più: `oe-copilot-ui`, quello del servizio Python `ui`, e dove c'era `oe-copilot-agente-ts`, quello dell'agente in TypeScript di prima, che ora è `agent`; se `.env` ha ancora una riga `AGENTE_URL`, si toglie prima, perché `--remove-orphans` toglie il container a cui il gateway punterebbe). Con la Action niente a mano: i secret non stanno in `.env` e compose ricreerebbe l'agente e il gateway con valori vuoti (4.7).
- **Quali prove girano dove.** Il job *Controlli* della Action (`.github/workflows/server.yml`) fa, in quest'ordine: la sintassi di `server/strumenti/*.py` e `*.sh`; `docker compose config`, con l'elenco dei servizi che deve essere `agent caddy-ui gateway schermo supervisore telefono` (nessuno sta dietro un profilo); la validazione del Caddyfile; il gateway (tipi, `npm test`, build); il supervisore (tipi, `npm test`, immagine avviata come in compose); l'agente (tipi, `npm test`, immagine avviata come in compose e confronto con `golden/agente.json`); lo schermo (tipi, build, prova di fumo, `npm test`, immagine); e il passo *Gateway, accesso e cancello*, con gateway, schermo, Caddy e agente veri davanti a un calendario inventato. Il job *Aggiorna il server* parte solo se i controlli passano, e solo su `main` (4.6). La Action non prova `app/` (`npm test` e `cargo test` si lanciano a mano: `app/README.md`, sezione Prove) e non lancia `controlla_dati_personali.py` (a mano, prima di un push). Le risposte e le sequenze registrate (`server/strumenti/golden/`, `server/supervisore/test/golden/`) sono dati di prova fissi: le diedero una sola volta le implementazioni tolte dal repository (l'agente Python, il servizio Python `ui`, il rilevatore Python e lo script bash di sincronizzazione), nessuno le rigenera, e una differenza è un comportamento cambiato, da leggere prima di toccare il file registrato.
- **Prove dell'agente** (`server/agente-ts/`, `npm test`, le lancia la Action): `test/golden-agente.test.ts` costruisce un `calendar.db` inventato come quello del telefono (serie con fusi e cambio dell'ora, occorrenze annullate e spostate, EXDATE, giorni interi, doppioni, inviti), avvia l'agente vero in HTTP su una porta libera e confronta le sue risposte con quelle registrate in `server/strumenti/golden/agente.json`, `POST /api/ics` compreso su un corpus di calendari (`server/strumenti/golden/ics/*.ics`: festività, serie a New York attraverso il cambio dell'ora, esclusioni, giorni interi, orari UTC, testo non valido). Gli altri file provano i pezzi uno per uno (`agente-unita.test.ts`: date, entità, link, percorsi, JSON, ordine dei testi), le richieste che nessun cliente normale manda e i modi di non partire (`agente-bordi.test.ts`) e il motore delle ricorrenze (`ricorrenze-*.test.ts`). Il passo *Agente in TypeScript, immagine e risposte* della Action rifà il confronto con `agente.json` sul container dell'immagine, come in compose (sola lettura, utente 10001, senza capacità), con `test/confronta-http.ts`. Le risposte registrate sono quelle che l'agente Python e le sue librerie diedero una sola volta, quando il Python c'era ancora (cronologia di git): nessuno le rigenera e sono dati di prova fissi, come `fixture.json`. Se una modifica all'agente cambia una risposta la prova mostra la differenza: è un comportamento cambiato, da leggere prima di toccare il file registrato.
- **Prove del supervisore** (`server/supervisore/test/`, `npm test`, le lancia la Action): `ciclo.test.ts` confronta, per ogni scenario (telefono spento o bloccato, schermo spento, pausa, Outlook nelle impostazioni, tocco ogni giro o spento, giri lunghi), la sequenza dei comandi al telefono, delle attese e dei battiti con quella che lo script bash tolto produsse una volta sola (`test/golden/ciclo-<scenario>.txt`, con un telefono inventato in `telefono-simulato.ts`); `differenziale.test.ts` confronta il rilevatore con le uscite registrate del rilevatore Python su 268 casi (sezione 6); `rilevatore.test.ts`, `rileva-accessi.test.ts` e `telefono.test.ts` provano i pezzi uno per uno (le regole del rilevatore, la freschezza di Outlook e dei calendari Google, il telefono bloccato, il client adb).
- **Prove del gateway con le risposte registrate** (`server/web-ts/test/golden-lettura.test.ts` e `golden-scritture.test.ts`, `npm test`, le lancia la Action): i metodi dell'API del gateway rispondono a una sequenza di chiamate di lettura (`get_config`, `get_personalizzazione`, `get_calendars`, `get_events`, `get_disponibilita`) con un agente finto che ripete le risposte di `golden/agente.json` in ogni stato (fermo, in pausa, caselle in errore, irraggiungibile), su una cartella dei dati costruita da `golden/ui/fixture.json`, e a una sequenza di modifiche (interruttori, gruppi, alias, calendari da link, eventi personali, configurazione), dopo ognuna delle quali si controlla lo stato: `config.json`, `local_events.json` e le copie dei calendari ICS. Risposte e stato devono essere uguali a quelli registrati in `golden/ui-lettura.json` e `golden/ui-scritture.json`. Quei due file li registrò il servizio Python `ui` con la stessa sequenza (lo script `prova_ui_golden.py` e il servizio sono stati tolti: l'ultimo commit che li contiene è `142e789`). Da allora sono dati di prova come `fixture.json`: nessuno li rigenera, e una differenza è un comportamento del gateway che è cambiato, da leggere prima di toccare il file.
- **Prove del secondo fattore** (`server/web-ts/test/secondo-fattore.test.ts`, `passkey.test.ts`, `avvio.test.ts`, `qr.test.ts`, `npm test`, le lancia la Action): utenti di prova a caso in un database in memoria, mai la password di nessuno. Coprono l'attivazione, l'accesso con il codice (generato da un TOTP di RFC 6238 scritto nella prova, non dalla libreria che lo controlla), il codice sbagliato e il blocco, i codici di recupero a uso singolo, la disattivazione, `off` identico a prima (le strade non ci sono e, con il codice già attivato, basta la password), l'avvio con ogni valore di `AUTH_2FA` anche su un database fatto dall'immagine di prima, la cerimonia delle passkey con un autenticatore in software (accesso, firma falsa, origine o nome del sito sbagliati, sfida riusata, contatore che non cresce) e il QR, riletto da un lettore indipendente. Il passo *Gateway, accesso e cancello* della Action prova la modalità `off` sul container vero (log di avvio, tabelle, strade 404, valore sbagliato che ferma l'avvio) e l'avvio con `passkey` dall'immagine.
- **Prove della condivisione del calendario** (`server/web-ts/test/ics-uscita.test.ts`, `condivisione.test.ts`, `condivisione-eventi.test.ts`, `condividi.test.ts`, `condivisione-importazione.test.ts`; `npm test`, le lancia la Action; da sole: `npx vitest run test/ics-uscita test/condivisione test/condividi`): senza rete e senza dati veri. Un agente finto risponde a qualunque finestra con gli eventi che la prova dichiara (`test/agente-condivisione.ts`) e un corpus di titoli, luoghi, organizzatori, indirizzi di posta, link per collegarsi e nomi di aziende non deve comparire in nessun punto del livello occupato (nel testo scritto a mano, nella lettura e attraverso il percorso). Coprono anche il testo ICS (a capo CRLF, righe piegate a 75 ottetti con caratteri a più ottetti, escape, ora UTC a cavallo dei cambi d'ora di marzo e ottobre 2026, giorni interi), gli identificativi, il confronto del testo segreto, il 404 uguale per ogni indirizzo che non vale, i 304, la memoria di un minuto, il conto degli indirizzi sbagliati, l'ultimo accesso, i metodi dell'API con le regole del gateway e il lato di chi importa. Il passo *Gateway, accesso e cancello* della Action prova il percorso sul container vero, davanti a un agente vero su un calendario inventato: 404, link creato con la sessione, `text/calendar`, 304, titoli e luoghi solo nel livello con i titoli (le note mai), revoca. L'agente di prova ha una fonte che non si legge: un link a tutti i calendari deve dare 503 con `Retry-After` e nessun calendario, uno a un gruppo senza quell'azienda risponde. A fine passo, con il gateway fermo, sette forme dell'indirizzo di un link (anche con l'indirizzo intero, anche con il testo segreto fuori da `/condividi/`) e l'intestazione `Referer` fanno rispondere 502 a Caddy, e nel suo log non c'è il testo segreto.
- **Prove dei loghi da indirizzo** (`server/web-ts/test/rete-sicura.test.ts`, `rete-sicura-ostile.test.ts`, `immagine.test.ts`, `immagine-ostile.test.ts`, `semaforo.test.ts`, `loghi.test.ts`, `loghi-api.test.ts`, `loghi-pagina.test.ts`, `npm test`, le lancia la Action): un server HTTP su 127.0.0.1 fa da sito dei loghi, nessuna rete vera. Coprono la guardia contro gli indirizzi non pubblici (ogni classe IPv4 e IPv6 e i modi di scriverla, porte, credenziali, schemi, reindirizzamenti verso la rete interna, un nome che cambia indirizzo a metà strada, le risposte del DNS che portano alla rete interna e lo stesso testo di un nome che non esiste, HTTPS con un certificato a perdere), i limiti di grandezza e di tempo (anche gli 8 secondi veri), un sito che manda il corpo un byte per volta (la memoria cresce di poco), i primi byte e la grandezza di ogni formato (troncata, 0, sopra il limite, un ICO con dentro un PNG enorme), gli SVG (quelli puliti, e più di cento modi di renderli attivi o di nasconderne il male: riferimenti numerici, commenti, CDATA, escape CSS, animazioni, prologo), il tempo di ogni famiglia di input che mandava il controllo in tempi lunghissimi (sotto 200 ms, e sotto 1 secondo a 512 KB), i posti per i collegamenti (60 richieste insieme: al più 4 socket aperti, 16 in coda, le altre *Troppe richieste*), la tabella `loghi`, l'unione con `personalizzazione.json`, gli indirizzi scritti a mano (scarico in secondo piano, barra rovesciata e tab, 7 giorni, nuovo tentativo dopo 5 minuti), il cambio di nome, la pulizia della cartella, i salvataggi insieme e i metodi dietro il cancello (401, 403, 415, 413). Il server di prova raggiunge 127.0.0.1 solo con `impostaReteDiProva` (`lib/core/uscita.ts`, riesportata da `loghi.ts`): nessun codice del gateway la chiama, nessuna variabile d'ambiente la attiva e in produzione solleva. Le prove HTTPS cercano `openssl` per fare un certificato a perdere e si saltano se non c'è (su Windows quello di Git, `C:\Program Files\Git\usr\bin`, messo nel `PATH`). Il passo *Gateway, accesso e cancello* della Action prova sul container vero che il metodo rifiuta 127.0.0.1 con il suo testo, che senza sessione risponde 401, che un file da 155 byte che prima fermava il controllo si rifiuta subito e che `/logos/` porta la sua politica. `ics-guardia.test.ts` ripete per `add_ics_calendar` le classi di indirizzi vietati, i reindirizzamenti, gli 8 MiB, il contenuto, la copia con 404 e 410 e i posti in comune con i loghi; `condivisione-importazione.test.ts`, `golden-lettura.test.ts` e `golden-scritture.test.ts` impostano il punto di prova per raggiungere il loro server su 127.0.0.1.
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
- **Link di condivisione del calendario** (4.10): stanno nella tabella `condivisioni` di `ui-dati/gateway.db` e non nei due file, quindi il ripristino qui sopra non li tocca; un database nuovo li perde e vanno ricreati, con gli indirizzi nuovi da mandare a chi li legge.

### Diagnosi rapida

| Sintomo | Diagnosi, sul server | Cosa fare |
|---|---|---|
| Pallino giallo *Telefono bloccato*, le cancellazioni arrivano in ritardo | `cat /opt/oe-copilot/stato/battito.json` (`bloccato` dice se la schermata di blocco è su, `telefono` se il telefono risponde, `pausa` se la sincronizzazione è ferma dal file `PAUSA`) e `docker logs --tail 20 oe-copilot-supervisore` | Sbloccare dalla pagina `/telefono/` (il PIN lo digita l'utente). Il blocco torna 15 minuti dopo l'ultimo input e il tocco ogni `TOCCO_OGNI` giri lo rimanda (4.5) |
| Dopo un aggiornamento il sito non risponde, e la Action è rossa con *il servizio oe-copilot-caddy non serve il sito* | `docker logs --tail 40 oe-copilot-caddy`, `docker inspect -f '{{.RestartCount}}' oe-copilot-caddy` e `curl -k --resolve <UI_HOST>:4445:127.0.0.1 https://<UI_HOST>:4445/login` | `docker compose up -d --force-recreate --no-deps caddy-ui`. Con un Caddyfile che non parte, tornare indietro solo con il Caddy (4.6). Alla prima installazione, senza certificato, rilanciare la Action dopo qualche minuto |
| Il link di condivisione risponde 503 | `docker logs oe-copilot-gateway 2>&1 \| grep condivisione` (la riga dice il motivo) e `curl -s -H "Authorization: Bearer $(sudo docker exec oe-copilot-agent printenv API_KEY)" http://127.0.0.1:3180/api/salute` (lo stato di ogni casella) | Una casella in `errore`: il suggerimento del suo pallino rosso dice cosa è fermo, e un accesso scaduto si rifà dalla pagina `/telefono/`. Un battito più vecchio di 15 minuti: il supervisore non gira (`docker logs oe-copilot-supervisore`). Più di 5000 eventi nella finestra: accorciare i giorni del link (4.10) |
| Un logo viene rifiutato | Il messaggio del pannello *Logo* dice il perché (l'elenco è in 4.7). Con *Non riesco a salvare il logo sul server*: `ls -ld /opt/oe-copilot/ui-dati/logos` e `docker logs --tail 50 oe-copilot-gateway 2>&1 \| grep 'logo: eccezione'` | La cartella deve essere dell'utente 10001: `sudo chown 10001:10001 /opt/oe-copilot/ui-dati/logos`. Con *Indirizzo non ammesso* o *L'indirizzo non risponde* l'indirizzo deve portare a un sito pubblico sulle porte 80 o 443 |
| Un passo dell'aggiornamento è rosso | Il testo su stderr del passo *Ricostruisci e riavvia* nel log della Action dice quale controllo è fallito. `cat /opt/oe-copilot/.versione` dice quale versione gira, e `docker ps --filter name=oe-copilot- --format '{{.Names}}: {{.Status}}'` lo stato dei container | Se il guasto è nel modulo `binder_linux`, nel telefono, nell'agente o nel supervisore lo script rimette la versione di prima. Se è nel gateway, nel Caddy o nello schermo non torna indietro niente (4.6). Il log di un servizio: `docker logs --tail 40 oe-copilot-<servizio>` |

---

## 8. Sicurezza e privacy

- **Il telefono sbloccato ha un solo cancello: l'accesso al sito.** Il servizio tiene il telefono sbloccato dopo il PIN (4.5), il servizio `schermo` non ha password e `adb` non chiede conferme (`ro.adb.secure=0`): chi supera l'accesso davanti a `/telefono/` comanda un telefono già sbloccato, con dentro le caselle di lavoro. Il PIN non fa da secondo fattore. Per questo la password dell'interfaccia va lunga e casuale (almeno 12 caratteri); il blocco per account (5 errori, 15 minuti) e la sessione (30 giorni, cookie HttpOnly) sono le difese del sito. Il secondo fattore dell'accesso (4.7), spento di default, ci aggiunge il codice di un'app di autenticazione o le passkey.
- **Il PIN c'è perché lo chiede un criterio di sicurezza aziendale**, imposto a Outlook da una delle aziende le cui caselle sono sul telefono: senza una password del dispositivo Outlook rimuove la casella (4.5). Non si toglie, non si cambia e nessuno script lo digita.
- **Il tocco sulla barra di stato aggira apposta il blocco a 15 minuti** dello stesso criterio (`TOCCO_OGNI`, 4.5): serve perché un telefono bloccato non applica cancellazioni e modifiche. Il prezzo è quello del primo punto. Per tornare al blocco automatico: `TOCCO_OGNI=0`.
- **Nessun segreto nel repository**, né nei file tracciati né nella storia di git (controllato l'11/10/2026 con ricerche di chiavi, token e password). Con l'aggiornamento automatico la chiave dell'API, la email e la password del proprietario e il segreto dei cookie stanno nei secret della Action, come i dati del collegamento SSH, e sul server arrivano nell'ambiente dei container e non in un file; in una prima installazione a mano stanno in `.env` (permessi 600). Nomi, domini, indirizzi, calendari, sigle e schede stanno in `ui-dati/personalizzazione.json` e in `agent/fonti.json`: file che il repository non ha mai contenuto e che `.gitignore` esclude. `server/strumenti/controlla_dati_personali.py` cerca i dati della personalizzazione nei file tracciati e fallisce se ne trova (si lancia a mano prima di un push, con il percorso del file).
- **Un link di condivisione vale come una chiave** (4.10): chi ha l'indirizzo `/condividi/<testo segreto>.ics` legge il calendario (solo gli orari occupati, o anche titoli e luoghi, secondo il livello) senza accesso. Si revoca o si cambia nelle Impostazioni, e subito dà 404. Il gateway non lo scrive nei log e il filtro del Caddyfile lo toglie anche da quelli di Caddy; resta nella cronologia del browser di chi lo usa.
- **Il gateway scarica da sé solo i loghi e i calendari da link, e solo da siti pubblici.** Sta sulla stessa macchina degli altri servizi e vicino ai metadati del cloud: per questo la guardia di `rete-sicura.ts` (4.7) rifiuta indirizzi della rete interna, di Docker e `169.254.169.254`, anche dietro un nome o un reindirizzamento, e i campi dell'indirizzo sono raggiungibili solo dopo l'accesso. Un calendario su un indirizzo privato non si può più sottoscrivere. Il tipo del file si decide dai primi byte (un logo, un calendario che comincia con `BEGIN:VCALENDAR`) e uno SVG con script non si accetta.
- **I login stanno nel telefono sul server**, non sul PC. I dati del telefono (`/opt/oe-copilot/telefono/data`) contengono i token degli accessi: proteggili come credenziali.
- **Niente esposto su internet** a parte l'interfaccia web: l'agente e l'adb ascoltano solo sul localhost del server; dal PC ci si arriva con un tunnel SSH (chiave SSH) e la chiave dell'API (sezione 5). Le eccezioni, facoltative, sono l'interfaccia nel browser (4.7) e lo schermo del telefono (4.8): HTTPS e una pagina di accesso davanti a una pagina che mostra i calendari e a una che comanda il telefono. Il servizio `schermo` in sé non è pubblicato e non ha password: ci arriva solo chi ha passato l'accesso del gateway.
- **I token non si leggono**: il rilevatore degli accessi guarda solo bandierine ed età di sincronizzazione, mai il contenuto di un token.
- Non estrarre token/cookie dai browser e non usare client ID di altre app per aggirare un consenso.
