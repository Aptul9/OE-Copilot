# OE Copilot, guida

Come si installa e si gestisce il server di OE Copilot (la "VM"), e perché è fatto così.

OE Copilot unisce le caselle di tutti gli account in un calendario solo, con un colore per azienda. Le legge un agente sul server, da un telefono Android virtuale. Il calendario si apre nel browser, dietro HTTPS e una pagina di accesso (4.7).

Indice:
1. [Perché è fatto così](#1-perché-è-fatto-così)
2. [Architettura](#2-architettura)
3. [Le app per Windows e Android](#3-le-app-per-windows-e-android)
4. [Installazione sul server](#4-installazione-sul-server)
5. [Prove dal PC: tunnel verso l'agente](#5-prove-dal-pc-tunnel-verso-lagente)
6. [Il rilevatore degli accessi scaduti](#6-il-rilevatore-degli-accessi-scaduti)
7. [Manutenzione e diagnostica](#7-manutenzione-e-diagnostica)
8. [Sicurezza e privacy](#8-sicurezza-e-privacy)

---

## 1. Perché è fatto così

- **Niente Microsoft Graph né Azure.** I tenant aziendali bloccano il consenso alle app di terze parti: ne basta uno per dover tenere un'altra strada.
- **Niente scraping di OWA.** Si rompe a ogni cambio della pagina.
- **Outlook su un telefono Android virtuale.** È un'app Microsoft di prima parte, già autorizzata in tutti i tenant. Gli accessi restano nel telefono sul server.
- **redroid.** Android in Docker, nativo sul kernel dell'host, senza emulazione.
- **Servizi Google sul telefono** (MindTheGapps, da `server/immagine-telefono/`). Outlook non esporta gli account Google nel calendario di Android.
- **Supervisore.** Android sincronizza al più ogni 15 minuti, e Outlook applica le cancellazioni solo in primo piano, a telefono sbloccato (4.5).
- **Rilevatore.** Un login scaduto ferma il calendario senza errori di lettura (sezione 6).
- **Tunnel SSH.** L'agente ascolta solo sul localhost. Da internet si raggiunge solo l'interfaccia web.

---

## 2. Architettura

```
 Browser ──HTTPS──► caddy-ui ──► gateway ──► agent ──► calendar.db del telefono (sola lettura)
                        │        (accesso)                  ▲
                        │                                   │
                        └─ /telefono/ ─► schermo ──adb──► telefono (redroid) ◄── oe-copilot-supervisore
                                                                                   (ogni 35 s)
```

- **telefono**: redroid con Outlook e i servizi Google, che scrivono nel calendario di Android.
- **agent** (`server/agente-ts/`): serve quel calendario con un'API a chiave, solo su `127.0.0.1:3180`, ed espande i calendari da link (`POST /api/ics`).
- **supervisore** (`server/supervisore/`): tiene aggiornato il telefono e scrive `stato/battito.json` e `stato/accessi.json`.
- **gateway** e **caddy-ui** (4.7): accesso, pagina e logica dell'app, con lo stato in `ui-dati/gateway.db`.
- **schermo** (4.8): il telefono nel browser.

Tutti stanno in `server/docker-compose.yml`, senza profili.

---

## 3. Le app per Windows e Android

Il client in `app/` (Tauri v2) è una finestra sul sito, con i promemoria scelti nel sito (4.9): su Windows finché è in esecuzione, su Android anche ad app chiusa. Su Windows aggiunge icona nell'area di notifica, avvio con Windows e avviso a comparsa. Costruzione, installazione e trappole: [`app/README.md`](app/README.md).

---

## 4. Installazione sul server

### Messa in servizio per un altro server

La prima installazione è a mano, poi aggiorna la Action del proprio repository:
1. Host con `binder_linux` (4.1), `ssh` e `gh` sul PC, un nome per il sito sulla 443 (4.7).
2. L'APK di Outlook nella release `apk`, con la sua riga in `server/apk/SHA256SUMS` (4.4).
3. Codice in `/opt/oe-copilot` (4.2), utente `oecopilot` (4.6), cartelle `stato/` (4.5) e `ui-dati/` (4.7).
4. `.env` con `UI_HOST` e `agent/fonti.json`, senza i quali `aggiorna_server.sh` si ferma.
5. Gli otto secret (4.6), poi un push su `main` o `gh workflow run Server --repo <proprietario>/<repository> --ref main`.
6. Login nelle app del telefono (4.4), caselle in `agent/fonti.json`, `sudo docker compose restart agent`. Il PIN se una casella lo impone (4.5).

### 4.1 Requisiti dell'host

- Linux ARM64 con Docker, il plugin `compose`, `rsync` e `curl`, e 2-4 GB di RAM. I servizi Google e l'APK di Outlook sono solo per arm64-v8a.
- Il fuso `Europe/Rome` è fisso nel gateway (`server/web-ts/src/lib/core/fuso.ts`), nell'agente (`TZ` nel Dockerfile) e sul telefono (`prepara_telefono.sh`): non c'è una variabile.
- Il modulo del kernel `binder_linux`. Su alcuni kernel cloud manca, e serve `linux-modules-extra`.
  ```bash
  grep -E 'CONFIG_ANDROID_BINDER_IPC|CONFIG_ANDROID_BINDERFS' "/boot/config-$(uname -r)"
  # devono essere =y o =m
  ```

Il modulo si carica all'avvio così:
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
`/opt/oe-copilot` non deve esistere, altrimenti `cp -a` crea `/opt/oe-copilot/server`.

I due file di configurazione, solo sul server:
```bash
# chiave dell'API: lunga e casuale
printf 'API_KEY=%s\n' "$(openssl rand -hex 32)" | sudo tee .env >/dev/null
sudo chmod 600 .env

# elenco delle caselle da leggere
sudo cp agent/fonti.esempio.json agent/fonti.json
sudoedit agent/fonti.json
```
La riga `API_KEY` serve solo a un'installazione a mano: con la Action la chiave arriva dai secret (4.7).

`fonti.json` ha una voce per casella:
```json
[
  {"tipo": "android", "id": "android::mario.rossi@azienda.example", "nome": "Azienda",
   "account": "mario.rossi@azienda.example", "calendari": ["Calendar", "Beta"]}
]
```
`account` è l'indirizzo come compare in Outlook, `nome` l'etichetta, `id` un identificativo univoco. Senza `calendari` si legge solo il calendario principale; con `calendari` quelli elencati per nome, **principale compreso**. Un calendario condiviso diventa un'azienda a sé con `azienda_per_cartella` e `aziende_inoltrate` ([README](README.md#personalizzazione)).

### 4.3 Avviare lo stack

Solo per una prima installazione a mano. Su un server con la Action i secret non stanno in `.env` (4.7).
```bash
sudo docker compose up -d telefono          # avvia 'telefono' (redroid)
sudo docker exec oe-copilot-telefono getprop sys.boot_completed    # ripeti finché risponde 1: il telefono è avviato
sudo docker compose up -d agent             # avvia 'agent', solo a telefono avviato
sudo docker ps                   # devono essere "Up" entrambi
```
L'agente va avviato a telefono pronto: altrimenti Docker crea vuota, e di root, la cartella del calendario di Android. Se il telefono non parte, quasi sempre manca `binder_linux`.

### 4.4 Configurare il telefono virtuale

**Impostazioni e app** (fuso, lingua, schermo acceso, Outlook, permessi) le mette lo script, a ogni aggiornamento: un telefono ricreato torna a posto, salvo i login.
```bash
sudo /opt/oe-copilot/strumenti/prepara_telefono.sh
```

**L'APK di Outlook** (arm64-v8a) sta nella release `apk`: supera il limite di GitHub per un file. Si installa solo quello elencato in `apk/SHA256SUMS`, con l'impronta giusta. A mano:
```bash
gh release download apk --repo <proprietario>/OE-Copilot --pattern 'Outlook-*.apk' --dir /opt/oe-copilot/apk
```
Versione nuova: `gh release upload apk <file>`, la riga di `sha256sum <file>` al posto della vecchia in `server/apk/SHA256SUMS`, push. Un telefono che ha già Outlook non lo reinstalla; si aggiorna a mano, e i login restano:
```bash
sudo docker cp /opt/oe-copilot/apk/<file> oe-copilot-telefono:/data/local/tmp/app.apk
sudo docker exec oe-copilot-telefono pm install -r /data/local/tmp/app.apk
```

**Schermo.** La pagina `/telefono/` (4.8), o [scrcpy](https://github.com/Genymobile/scrcpy) dal PC con un tunnel SSH verso `127.0.0.1:5555` e il file `/opt/oe-copilot/telefono/PAUSA` creato per la durata: senza, il supervisore riporta Outlook davanti.

**Login**, sullo schermo del telefono. Password e MFA le digita l'utente.
1. **Caselle Microsoft**, in Outlook: per ogni casella *Impostazioni > Calendario > Sincronizza calendari*. È per account, e le caselle nuove partono spente. Dei calendari secondari arrivano solo quelli spuntati in Outlook.
2. **Calendari Google**, se manca il link iCal segreto: *Impostazioni > Password e account > Aggiungi account > Google*, poi la spunta in *Calendari da sincronizzare* dell'app Calendario. Un dispositivo "non certificato" si registra su https://www.google.com/android/uncertified/.
3. Le caselle in `agent/fonti.json`, poi `sudo docker compose restart agent`.

### 4.5 Il servizio di sincronizzazione

Il servizio `supervisore` (`server/supervisore/`) parla col telefono con adb su `telefono:5555`. A ogni giro, circa ogni 35 secondi (`OGNI`, 30, più il lavoro):
- forza la sincronizzazione di Outlook, e di Google ogni 4 giri (`GOOGLE_OGNI`);
- riaccende lo schermo se nessuna pagina del telefono è aperta, e riporta Outlook sulla sua schermata principale;
- scrive `stato/battito.json`, e ogni 2 giri (`ACCESSI_OGNI`) lancia il rilevatore (sezione 6);
- ogni 10 giri (`TOCCO_OGNI`) tocca la barra di stato e rimanda il blocco; `TOCCO_OGNI=0` in `docker-compose.yml` lo spegne.

Con un file `PAUSA` fresco (`telefono/PAUSA`, o `stato/schermo/PAUSA` della pagina del telefono) non porta Outlook davanti.

Gira come l'utente 10080, quello di Outlook sul telefono, l'unico oltre a root che ne legge database e log. `stato/` deve essere suo, `stato/schermo` dell'utente 10001:
```bash
cd /opt/oe-copilot
sudo chown 10080:10080 stato
sudo install -d -o 10001 -g 10001 -m 755 stato/schermo       # solo se manca (di solito c'è già)
docker compose up -d --build --no-deps supervisore
docker inspect -f '{{.State.Health.Status}}' oe-copilot-supervisore       # healthy entro circa un minuto
cat stato/battito.json; stat -c '%y' stato/accessi.json                    # battito ogni 35 secondi, accessi ogni 70
```

**PIN.** Lo impone il criterio di sicurezza di un'azienda, che blocca anche il telefono 15 minuti dopo l'ultimo input (il tocco lo rimanda). Non si toglie, altrimenti Outlook rimuove la casella. Si imposta con `sudo docker exec oe-copilot-telefono cmd lock_settings set-pin <PIN>`: le schermate del PIN escono nere nella pagina del telefono. Lo sblocco lo digita l'utente.

Verifica dell'agente, dal server:
```bash
curl -s -H "Authorization: Bearer $(sudo docker exec oe-copilot-agent printenv API_KEY)" \
  http://127.0.0.1:3180/api/calendari | python3 -m json.tool | head
```

### 4.6 Aggiornamento automatico

La Action `.github/workflows/server.yml` parte a ogni push su `main` che tocca `server/` o il workflow, o con `gh workflow run Server --repo danfortu/OE-Copilot --ref main`. Dopo i *Controlli* (sezione 7), *Aggiorna il server*:
1. salva il codice in esecuzione in `/var/tmp/oe-copilot-prima.tgz`;
2. scarica l'APK di Outlook e ne controlla l'impronta;
3. copia `server/` con rsync, senza toccare `.env`, `agent/fonti.json`, `telefono/`, `stato/`, `ui-dati/` e `.versione`;
4. lancia `strumenti/aggiorna_server.sh avvia <sha>` con i secret sullo standard input: telefono (solo se cambia la sua configurazione), `prepara_telefono.sh`, agente, supervisore, poi la prova di agente e battito. Se qualcosa fallisce rimette la versione di prima;
5. avvia interfaccia (con `UI_HOST`) e schermo: un loro guasto rende rossa la Action, senza tornare indietro.

**Trappola: il Caddy.** `docker compose up` dà 0 anche con un container che si riavvia in continuazione, quindi lo script vuole 200 da `curl -k --resolve <UI_HOST>:4445:127.0.0.1 https://<UI_HOST>:4445/login` e `RestartCount` 0. Alla prima installazione fallisce finché Caddy non ha il certificato: si rilancia dopo qualche minuto. Il Caddyfile di prima si rimette così:
```bash
cd /opt/oe-copilot && tar -xzf /var/tmp/oe-copilot-prima.tgz ./caddy-ui && docker compose up -d --no-deps --build caddy-ui
```
`--no-deps` è obbligatorio, altrimenti compose ricrea il gateway senza i secret.

**Trappola: quale codice gira.** Lo dicono `Created` e `StartedAt` del container (`docker inspect -f '{{.Created}} {{.State.StartedAt}}' oe-copilot-agent`) e `.versione`. Gli ID delle immagini no: con containerd sono digest diversi.

**Utente della Action**, una volta:
```bash
sudo useradd -m -s /bin/bash -G docker oecopilot
# il codice è dell'utente della Action; telefono/ resta di root, stato/ del 10080 (4.5), ui-dati/ del 10001 (4.7)
sudo chown oecopilot:oecopilot /opt/oe-copilot /opt/oe-copilot/.env
cd /opt/oe-copilot && sudo chown -R oecopilot:oecopilot agent agente-ts apk caddy-ui immagine-telefono schermo-web strumenti supervisore web-ts docker-compose.yml .env.esempio
```
La sua chiave pubblica va in `/home/oecopilot/.ssh/authorized_keys`. I secret del repository (`gh secret set <NOME> --repo <proprietario>/<repository>`): `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`, `DEPLOY_KNOWN_HOSTS` (da `ssh-keyscan -t ed25519 <server>`), `ADMIN_EMAIL`, `ADMIN_PASSWORD`, `API_KEY` e `BETTER_AUTH_SECRET` (4.7).

### 4.7 L'interfaccia nel browser

- **`gateway`** (`server/web-ts/`, Next.js e Better Auth, `127.0.0.1:3190`): accesso, pagina, API (`POST /api/<metodo>`) e loghi. Senza sessione le pagine vanno a `/login` e l'API dà 401. Restano aperti `/login`, `/api/auth/*`, `/salute`, `/disponibilita`, `/condividi/...` e i file statici come `/sw.js`.
- **`caddy-ui`**: HTTPS, solo su `127.0.0.1:4445`.
- **`ui-dati/`**: `gateway.db`, cache, loghi e `personalizzazione.json`.

Messa in funzione, poi `UI_HOST=<nome del sito>` in `.env`:
```bash
# dati dell'interfaccia: i container li scrivono come utente 10001, senza privilegi
sudo install -d -o 10001 -g 10001 -m 700 /opt/oe-copilot/ui-dati
# facoltativo: dal modello personalizzazione.esempio.json, nella radice del repository (non in server/)
sudo install -o 10001 -g 10001 -m 600 personalizzazione.json /opt/oe-copilot/ui-dati/
```

**Trappola: lo stato sta nel database** (`gateway.db`, tabelle `stato_config` ed `eventi_locali`). `config.json` e `local_events.json` sono copie riscritte a ogni modifica e mai rilette: cambiarle non serve (sezione 7).

**Accesso.** Il proprietario nasce a ogni avvio da `ADMIN_EMAIL` e `ADMIN_PASSWORD`; con una email nuova il vecchio utente va tolto da `gateway.db` (tabelle `session`, `account`, `user`). Il cookie dura 30 giorni. Cinque password sbagliate bloccano l'account 15 minuti, per tutti: dietro sslh i client hanno lo stesso indirizzo.

**Secret.** `ADMIN_EMAIL`, `ADMIN_PASSWORD` (almeno 12 caratteri, senza `$`), `API_KEY` e `BETTER_AUTH_SECRET` (almeno 32 caratteri, `openssl rand -hex 32`) stanno nei secret della Action, e compose li legge dall'ambiente prima che da `.env`. Si cambiano con `gh secret set` e un nuovo aggiornamento. Senza la Action stanno in `.env`.

**Trappola: niente `docker compose up` a mano con la Action.** Ricrea l'agente con `API_KEY` vuota, e non parte. Ricrea il gateway senza `BETTER_AUTH_SECRET`, e si ferma con "BETTER_AUTH_SECRET deve essere impostata". Dopo una modifica a `.env` serve un nuovo aggiornamento: `docker restart` non rilegge l'ambiente.

**HTTPS.** Caddy ottiene il certificato da solo. Con la 443 libera bastano le porte `443:443` e `80:80` su `caddy-ui`, lasciando `127.0.0.1:4445:443` per il controllo del deploy. Con sslh sulla 443 si aggiunge a `/etc/sslh.cfg`, sopra la voce che prende tutto il resto:
```
{ name: "tls"; host: "127.0.0.1"; port: "4445"; sni_hostnames: [ "oe.<IP con i trattini>.sslip.io" ]; },
```
e si riavvia sslh. Il certificato arriva col TLS-ALPN; il primo tentativo HTTP-01 fallisce, ed è previsto.

#### Secondo fattore dell'accesso

`AUTH_2FA` in `.env`: `off` (predefinito, solo password, `/sicurezza` risponde 404), `totp` (dopo la password il codice di un'app di autenticazione o un codice di recupero) o `passkey` (anche le passkey, legate all'indirizzo del sito). Un altro valore ferma il gateway con `AUTH_2FA non valido`. Dopo un nuovo aggiornamento il codice si attiva da *Impostazioni > Sicurezza > Gestisci l'accesso*; i 10 codici di recupero si vedono una volta sola. Cambiare `BETTER_AUTH_SECRET` invalida i codici attivati.

**Recupero.** Un codice di recupero (*Usa un codice di recupero*). Altrimenti `AUTH_2FA=off` e un nuovo aggiornamento, e basta la password:
```bash
cd /opt/oe-copilot
sudo sed -i 's/^AUTH_2FA=.*/AUTH_2FA=off/' .env
# poi, dal PC: gh workflow run Server --repo danfortu/OE-Copilot --ref main   (senza la Action: sudo docker compose up -d gateway, con i secret in .env o nell'ambiente)
docker logs --tail 5 oe-copilot-gateway    # a deploy finito deve dire: secondo fattore dell'accesso (AUTH_2FA): off
```
Per ripartire da zero col codice, a gateway fermo (le passkey restano):
```bash
docker stop oe-copilot-gateway
docker run --rm --user 10001:10001 -v /opt/oe-copilot/ui-dati:/dati oe-copilot-gateway \
  node -e "new (require('better-sqlite3'))('/dati/gateway.db').exec('DELETE FROM twoFactor; UPDATE user SET twoFactorEnabled = 0')"
docker start oe-copilot-gateway
```

#### Calendari, gruppi e disponibilità

- **Gruppi** (*Gestisci calendari*, voce `gruppi_calendari`): un pulsante nella barra lascia accesi solo i calendari del gruppo.
- **Account uniti** (voce `account_aliases`): più account sotto un nome solo. Vince su `nomi_per_indirizzo` e `nomi_per_dominio`.
- **Secondario** (voce `secondari_scelti`): la voce va sotto *Altri calendari*. Vince su `interfaccia.calendari_secondari`.
- **Disponibilità pubblica**: `https://<UI_HOST>/disponibilita`, senza accesso. Solo blocchi "Occupato", da tutti i calendari, anche spenti, senza giorni interi. `?gruppo=<nome>` la limita a un gruppo.

#### Loghi e calendari da link

**Loghi.** Voce `interfaccia.loghi` di `personalizzazione.json`: un file in `ui-dati/logos/` (`"logos/alfa.png"`) o un indirizzo, che il gateway scarica e rinnova ogni 7 giorni. Il pulsante *Logo* di *Gestisci calendari* sceglie un logo per voce, e vince sul file. Il browser non carica mai loghi da altri siti.

**Calendari da link.** *Gestisci calendari > Aggiungi un calendario da link*. Si rileggono ogni 30 minuti; una risposta 404 o 410 toglie la copia.

**Trappola: solo siti pubblici.** Il gateway scarica (`server/web-ts/src/lib/core/rete-sicura.ts`) solo `http` o `https` sulle porte 80 e 443, da nomi con soli indirizzi pubblici: niente loopback, reti private, link-local (`169.254.169.254`) o CGNAT, anche dopo un reindirizzamento. Un logo: al più 512 KB e 4096 pixel per lato, SVG senza script. Un calendario: al più 8 MiB, che comincia con `BEGIN:VCALENDAR`, con Content-Type assente o `text/calendar`, `text/plain`, `application/octet-stream` (`application/ics` è rifiutato). Vicino agli 8 MiB può superare gli 8.000.000 byte che l'agente accetta.

#### Personalizzazione

Nomi delle aziende, ordine dei calendari, sigle, loghi e schede Timesheet stanno in `ui-dati/personalizzazione.json`, dal modello `personalizzazione.esempio.json`. Le voci sono nel [README](README.md#personalizzazione). Senza il file le aziende prendono il nome dal dominio della casella. Il gateway rilegge il file quando cambia.

### 4.8 Lo schermo del telefono nel browser

Il servizio `schermo` (`server/schermo-web/`, da ws-scrcpy) mostra e comanda il telefono: pulsante *Telefono*, pagina `/telefono/`.
- **Trappola: nessuna password.** Il servizio non ha porte pubblicate né password, e adb non chiede conferme (`ro.adb.secure=0`). L'unico cancello è Caddy, che prima di passare `/telefono/*` chiede `/api/verifica-sessione` al gateway (`forward_auth`).
- La tastiera arriva come caratteri, le accentate come accento morto più lettera, Ctrl+V incolla; €, £ ed emoji no.
- Una scheda aperta riscrive ogni 10 secondi `stato/schermo/PAUSA` e ferma il supervisore; dopo 60 secondi il file non conta più.
- Il server scrcpy resta nel telefono anche a servizio fermo: *Kill server* nella pagina `/telefono/` lo ferma.
- Prova senza telefono: `cd server/schermo-web && npm ci --ignore-scripts && npm run dist && npm run smoke && npm test`.

### 4.9 I promemoria

Si scelgono nel sito, *Impostazioni > Promemoria*, e stanno sul server (voce `promemoria`, metodi `get_promemoria` e `set_promemoria`). L'app per Windows le rilegge ogni minuto, quella per Android ogni 15 minuti e quando passa in secondo piano.
- **Quando**: 1 ora, 30, 15, 10 o 5 minuti prima, *All'inizio*, e altri minuti in un campo. Al più 8 valori da 0 a 1440 (0 è all'inizio); di norma 10.
- **Dove**: nell'app per Windows *Notifica di Windows* (il toast, una per riunione con il logo dell'azienda, che sparisce all'inizio) e *Avviso a comparsa* (in basso a sinistra, visibile anche con *Non disturbare*). In un browser normale *Notifiche del browser*, che iscrive o toglie quel browser. L'app per Android avvisa sempre, anche chiusa.
- **Contenuto**, per tutti i canali: *Completo* (titolo e luogo) o *Solo ora e azienda* (l'avviso a comparsa mostra logo e ora).
- **Suono**: solo l'app per Windows, tra i suoni di sistema. Lo fa l'avviso a comparsa; ad avviso spento, la notifica.

Le voci degli avvisi le calcola il nucleo delle app (`app/src-tauri/src/promemoria.rs`): su Windows avvisa lo script della pagina, finché l'app è in esecuzione; su Android un lavoro in background programma un allarme per avviso, che parte anche ad app chiusa (`app/README.md`). Le notifiche del browser le manda il server, anche a pagina chiusa, con un giro ogni `PROMEMORIA_INTERVALLO_S` secondi (di norma 30). Un avviso per riunione e anticipo; giorni interi e calendari spenti non avvisano. Una cancellazione fatta in Outlook arriva a telefono sbloccato: prima la riunione può avvisare.

**Notifiche del browser.** Il testo passa cifrato (RFC 8291) dal servizio di notifiche del browser. Le WebView delle app non hanno notifiche web, quindi nell'app questo interruttore non c'è. Servono le chiavi VAPID:
   ```bash
   node -e "const c=require('crypto');const j=c.generateKeyPairSync('ec',{namedCurve:'P-256'}).privateKey.export({format:'jwk'});console.log('VAPID_PUBLIC_KEY='+Buffer.concat([Buffer.from([4]),Buffer.from(j.x,'base64url'),Buffer.from(j.y,'base64url')]).toString('base64url'));console.log('VAPID_PRIVATE_KEY='+j.d)"
   ```
Le due righe vanno nel `.env` della VM con `VAPID_SUBJECT=mailto:<indirizzo>` (o `https://<sito>`), poi un deploy con la Action `Server`. Senza i tre valori, o in un browser senza notifiche, l'interruttore non compare. La chiave privata e gli indirizzi delle iscrizioni (tabella `push_iscrizioni`, al più 20 browser) sono segreti e non finiscono nei log. Prove: `cd server/web-ts && npx vitest run test/promemoria`.

### 4.10 Condividere il calendario con un altro OE Copilot

Un link ICS per persona, `https://<UI_HOST>/condividi/<testo segreto>.ics`. L'altro OE Copilot lo aggiunge con *Gestisci calendari > Aggiungi un calendario da link* e lo rilegge ogni 30 minuti.
- **Gestione**: *Impostazioni > Condivisione > Gestisci i link*. Per ogni link: livello, cosa condividere, giorni indietro (0-30, di norma 7) e avanti (1-365, di norma 90). Al più 50 link, nella tabella `condivisioni`.
- **Livelli**: *Occupato* dà solo gli orari; *Con titoli* aggiunge titolo e luogo, mai descrizione, partecipanti o link per collegarsi.
- **Cosa esce**: i calendari accesi, o un gruppo. Mai quelli ricevuti da altri (link ICS, festività, altre condivisioni).
- **Revoca** e **Cambia indirizzo** valgono subito: l'indirizzo vecchio dà 404, e chi lo legge toglie la copia.
- **Trappola: 503, mai vuoto.** Agente fermo, casella in errore, gruppo sparito o più di 5000 eventi danno 503 con `Retry-After`: un calendario vuoto vorrebbe dire "libero". Il motivo sta nel log del gateway.
- **Un link è una chiave.** Il gateway non lo scrive nei log, e il filtro del Caddyfile lo toglie da quelli di Caddy (`/condividi/REDATTO`). In `condivisioni` sta in chiaro: una copia di `ui-dati/` va protetta.

---

## 5. Prove dal PC: tunnel verso l'agente

L'agente ascolta solo su `127.0.0.1:3180` del server e sulla rete di Docker. Dal PC si interroga con un tunnel SSH.
1. **Chiave SSH**, senza password, con un alias in `~/.ssh/config`:
   ```
   Host oecopilot
       HostName  198.51.100.10
       User      mario
       IdentityFile ~/.ssh/id_ed25519
   ```
   L'OpenSSH di Windows rifiuta una chiave leggibile da altri utenti ("UNPROTECTED PRIVATE KEY FILE"), come succede in OneDrive: va tenuta in `%USERPROFILE%\.ssh`, con `icacls <chiave> /inheritance:r /grant:r "%USERNAME%:R"`.
2. **Tunnel**, lasciato aperto: `ssh -N -L 13180:127.0.0.1:3180 oecopilot`.
3. **Chiamata**, con la chiave letta dall'ambiente dell'agente e mai stampata:
   ```bash
   CHIAVE=$(ssh oecopilot "docker exec oe-copilot-agent printenv API_KEY")
   curl -s -H "Authorization: Bearer $CHIAVE" http://127.0.0.1:13180/api/calendari
   ```

### L'agente

`server/agente-ts/` legge in sola lettura il `calendar.db` del telefono e `agent/fonti.json`. Gira come utente 10001, in sola lettura e senza capacità. Sorgenti e prove: [`server/agente-ts/README.md`](server/agente-ts/README.md).

**Trappola: il gruppo dei file del calendario.** I file di `calendar.db` sono del gruppo 10019, che entra nel container con `group_add`. Se Android cambia il numero, le caselle danno "Permission denied". Il numero lo dà `sudo stat -c %g /opt/oe-copilot/telefono/data/data/com.android.providers.calendar/databases/calendar.db`; va in `.env` come `CALENDARIO_GID=<numero>`, poi un nuovo aggiornamento.

**Trappola: le occorrenze annullate** (`annullata()` in `server/agente-ts/src/android.ts`). Una riga di una serie con `eventStatus` 2 annulla l'occorrenza, tranne nelle serie copiate da un flusso (`isOrganizer` 1, `hasAttendeeData` 0): Outlook le segna così anche se sono riunioni normali, e quelle righe prendono la durata della serie. Un annullamento vero ha un invitato, o il titolo "Canceled:" o "Annullato:", che il gateway nasconde.

---

## 6. Il rilevatore degli accessi scaduti

Il rilevatore (`server/supervisore/src/rilevatore/`) scrive ogni 2 giri in `stato/accessi.json` il pallino di ogni casella, senza leggere token.
- **Rosso** con la bandierina `needsReauth` di Outlook ("Accesso scaduto sul telefono").
- Outlook che non conferma la sincronizzazione in `calendarSync.log`: **giallo** oltre `OUTLOOK_GIALLO_S` (600 secondi), **rosso** oltre `OUTLOOK_ROSSO_S` (1800).
- **Giallo** a telefono bloccato, e senza push FCM con Outlook non in primo piano.
- Calendari Google, da `dumpsys content`: senza successi **giallo** oltre `GOOGLE_GIALLO_S` (900 secondi), **rosso** oltre `GOOGLE_ROSSO_S` (2700). `OTHER FAILURE internal-error` dell'adattatore dei contatti non conta.

---

## 7. Manutenzione e diagnostica

- **Login scaduto**: dalla pagina `/telefono/`, poi si chiude la scheda.
- **Log**: `docker logs oe-copilot-<servizio>`. Segni di vita in `stato/battito.json` e `stato/accessi.json`.
- **Senza la Action**: ricopiare `server/` lasciando `.env`, `agent/fonti.json`, `telefono/`, `stato/` e `ui-dati/`, poi `sudo docker compose up -d --build --remove-orphans`, con i secret in `.env`.
- **Prove della Action** (job *Controlli*): sintassi, `docker compose config` (l'elenco dei servizi è scritto nel workflow), Caddyfile, tipi, `npm test` e immagini dei servizi, poi *Gateway, accesso e cancello* sui container veri. `app/` e `controlla_dati_personali.py` restano a mano.
- **Risposte registrate** (`server/strumenti/golden/`, `server/supervisore/test/golden/`): dati fissi, mai rigenerati. Una differenza è un comportamento cambiato.
- **Backup**: `ui-dati/` e `/opt/oe-copilot/telefono/data`, che contiene i login. Lo snapshot del telefono si fa in `docker pause`, per pochi secondi.
- **Reimportare `config.json` e `local_events.json`** (dopo un ripristino o una correzione a mano), a gateway fermo. I link di condivisione restano.
  ```bash
  docker stop oe-copilot-gateway
  docker run --rm --user 10001:10001 -v /opt/oe-copilot/ui-dati:/dati oe-copilot-gateway \
    node -e "new (require('better-sqlite3'))('/dati/gateway.db').exec('DROP TABLE stato_config; DROP TABLE eventi_locali; DROP TABLE stato_meta')"
  docker start oe-copilot-gateway
  ```

### Diagnosi rapida

| Sintomo | Diagnosi, sul server | Cosa fare |
|---|---|---|
| Pallino giallo *Telefono bloccato* | `cat /opt/oe-copilot/stato/battito.json` e `docker logs --tail 20 oe-copilot-supervisore` | Sbloccare dalla pagina `/telefono/` |
| La Action dice *il servizio oe-copilot-caddy non serve il sito* | `docker logs --tail 40 oe-copilot-caddy` e `docker inspect -f '{{.RestartCount}}' oe-copilot-caddy` | `docker compose up -d --force-recreate --no-deps caddy-ui`, o il Caddyfile di prima (4.6) |
| Un link di condivisione risponde 503 | `docker logs oe-copilot-gateway 2>&1 \| grep condivisione` e `curl -s -H "Authorization: Bearer $(sudo docker exec oe-copilot-agent printenv API_KEY)" http://127.0.0.1:3180/api/salute` | Casella in `errore`: lo dice il suo pallino. Più di 5000 eventi: meno giorni nel link |
| *Non riesco a salvare il logo sul server* | `ls -ld /opt/oe-copilot/ui-dati/logos` | `sudo chown 10001:10001 /opt/oe-copilot/ui-dati/logos` |
| Un passo dell'aggiornamento è rosso | Lo stderr del passo *Ricostruisci e riavvia*, `cat /opt/oe-copilot/.versione` e `docker ps --filter name=oe-copilot- --format '{{.Names}}: {{.Status}}'` | Guasto in telefono, agente o supervisore: lo script è già tornato indietro. In gateway, Caddy o schermo no (4.6) |

---

## 8. Sicurezza e privacy

- **Un solo cancello: l'accesso al sito.** Il telefono resta sbloccato (4.5), `schermo` non ha password e adb non chiede conferme: chi entra nel sito comanda il telefono. La password va lunga e casuale; il secondo fattore (4.7) aggiunge un codice.
- **Nessun segreto nel repository.** Le credenziali stanno nei secret della Action, o in `.env` (permessi 600) con le chiavi VAPID. `personalizzazione.json` e `agent/fonti.json` sono esclusi da `.gitignore`.
- **I login stanno nel telefono**: `/opt/oe-copilot/telefono/data` contiene i token degli accessi.
- **Su internet c'è solo l'interfaccia web.** Agente e adb ascoltano solo sul localhost.
- Non si estraggono token o cookie dai browser e non si usano client ID di altre app per aggirare un consenso.
