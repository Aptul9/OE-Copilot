# OE Copilot

OE Copilot unisce in un calendario solo i calendari di tutti gli account, aziendali e personali, con un colore per azienda. Li legge un agente sul server, da un telefono Android virtuale con Outlook e i servizi Google: niente registrazione su Azure, niente consenso dell'amministratore. Il calendario si apre nel browser, dietro HTTPS e una pagina di accesso (4.7), o nelle app per Windows e Android (sezione 3). C'è anche una sezione Timesheet con i gestionali delle aziende.

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

- **Niente registrazioni su Azure né consensi amministratore.** I tenant aziendali bloccano il consenso alle app di terze parti.
- **Niente scraping di OWA né emulatore Android.** L'agente usa direttamente le API REST di Outlook (`https://outlook.office.com/api/v2.0`) con il client OAuth2 di prima parte di Outlook Mobile (`27922004-5251-4030-b22d-91ecd9a37ea4`) e l'API Google Calendar v3, rinnovando in automatico i token in `stato/tokens.json`.
- **Caselle dove Outlook Mobile è respinto** (accesso condizionale, `AADSTS53003`). L'agente usa il client web di Outlook (`9199bf20-a13f-4107-85dc-02114787ef48`) con la sessione Microsoft di un browser già entrato (cookie ESTS, formato `session.json` di teamsrelay): autorizzazione silenziosa `prompt=none` con PKCE, refresh token di 24 ore, poi di nuovo dalla sessione. Si importa una volta con `POST /api/auth/completa` `{"provider": "microsoft", "sessione": <session.json>, "tenant": "<tenant id>"}` (dal gateway: `completa_accesso_casella` con gli stessi campi); prima di salvare l'agente controlla che tornino la stessa casella, la risorsa Outlook e il permesso sui calendari. Quando anche la sessione finisce la casella va in errore: si rientra da Outlook web e si importa di nuovo.
- **Caselle dal browser.** La finestra *Account* (pulsante in fondo alla barra a sinistra, o *Gestisci calendari*) elenca a sinistra le caselle con il loro stato; la freccia accanto a una casella mostra le aziende inoltrate lì, e quella accanto a *Da link e personali* i suoi calendari. Ogni casella e ogni calendario ha la sua pagina: il logo (si tocca il cerchio per cambiarlo, o ci si trascina un'immagine), l'interruttore per questo dispositivo, il posto nella barra e i gruppi. *Aggiungi account* fa l'accesso Microsoft (codice) o Google; *Accedi di nuovo* compare solo quando un accesso è scaduto; *Rimuovi account* smette di leggere la casella e cancella l'accesso salvato (`rimossi` in `stato/tokens.json`, vale anche per le caselle di `fonti.json`, finché non si rifà l'accesso).
- **Tunnel SSH.** L'agente ascolta solo sul localhost. Da internet si raggiunge solo l'interfaccia web.

---

## 2. Architettura

```
 Browser ──HTTPS──► caddy-ui ──► gateway ──► agent ──HTTPS──► Outlook REST v2.0 / Google Calendar v3
                                 (accesso)     │
                                               ▼
                                        stato/tokens.json
```

- **agent** (`server/agente-ts/`): rinnova i token in `stato/tokens.json`, legge i calendari Microsoft e Google via API, li serve con un'API a chiave solo su `127.0.0.1:3180`, ed espande i calendari da link (`POST /api/ics`).
- **gateway** e **caddy-ui** (4.7): accesso, pagina e logica dell'app, con lo stato in `ui-dati/gateway.db`, più l'endpoint MCP `/mcp` per leggere e inviare le email delle caselle Outlook.
- **Google communication**: native TypeScript Gmail and Google Chat APIs in the agent, with separate credential storage and confirmed writes. API contract and PC import: [`server/agente-ts/GOOGLE.md`](server/agente-ts/GOOGLE.md). No additional UI screens or MCP tools are included.

Tutti e tre i servizi stanno in `server/docker-compose.yml`, senza profili.

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

- Linux ARM64 o x86_64 con Docker, il plugin `compose`, `rsync` e `curl`, e 2-4 GB di RAM. L'immagine del telefono prende i servizi Google dell'architettura della VM; l'APK di Outlook deve essere della stessa ABI del telefono (`arm64-v8a` o `x86_64`, 4.4).
- Kernel con il meta `linux-image-generic` (non `linux-virtual`, che hanno alcune VPS): solo così `linux-modules-extra`, dove sta `binder_linux`, segue gli aggiornamenti del kernel.
- **Trappola: il telefono spegne il caricamento automatico dei moduli dell'host.** Il container è `privileged` e all'avvio Android scrive `/proc/sys/kernel/modprobe` vuoto, che non è per namespace: da lì il kernel della VM non carica più moduli da solo. Docker allora non riesce a pubblicare le porte di un container nuovo (`Unable to enable DNAT rule`, `Extension udp revision 0 not supported`). Si caricano al boot i moduli netfilter di Docker (`xt_nat`, `xt_tcpudp`, `xt_addrtype`, `xt_conntrack`, `xt_MASQUERADE`, `xt_comment`, `xt_multiport`, `xt_set`, `br_netfilter`, `nf_nat` in `/etc/modules-load.d/`) e un timer di systemd rimette `sysctl -w kernel.modprobe=/sbin/modprobe` ogni 5 minuti.
- `sslh` da apt porta con sé `apache2`, che prende la porta 80: `apt-get install --no-install-recommends sslh`, o `apache2` va tolto.
- Il fuso `Europe/Rome` è fisso nel gateway (`server/web-ts/src/lib/core/fuso.ts`), nell'agente (`TZ` nel Dockerfile) e sul telefono (`prepara_telefono.sh`): non c'è una variabile. La pagina e l'avviso dell'app mostrano gli orari nel fuso del dispositivo; il testo delle notifiche del browser, scritto dal server, resta in ora di Roma.
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
`account` è l'indirizzo come compare in Outlook, `nome` l'etichetta, `id` un identificativo univoco. Senza `calendari` si legge solo il calendario principale; con `calendari` quelli elencati per nome, **principale compreso**. Un calendario condiviso diventa un'azienda a sé con `azienda_per_cartella` e `aziende_inoltrate` ([Personalizzazione](#personalizzazione)).

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

**L'APK di Outlook** sta nella release `apk`: supera il limite di GitHub per un file. Il nome porta l'ABI, `Outlook-<abi>-<versione>.apk`, e `prepara_telefono.sh` prende quello di `ro.product.cpu.abi` del telefono (`arm64-v8a` o `x86_64`). Si installa solo quello elencato in `apk/SHA256SUMS`, con l'impronta giusta. A mano:
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

Gira come l'utente di Outlook sul telefono, l'unico oltre a root che ne legge database e log: 10080, o `OUTLOOK_UID` in `.env`. Il numero lo assegna Android all'installazione e su un telefono nuovo può cambiare (sul telefono x86_64 è 10090, e il calendario è del gruppo 10020): si legge con `sudo stat -c %u /opt/oe-copilot/telefono/data/data/com.microsoft.office.outlook`. Con un numero sbagliato il rilevatore scrive `EACCES, utente 10080`. `stato/` deve essere suo, `stato/schermo` dell'utente 10001:
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
cd /opt/oe-copilot && sudo chown -R oecopilot:oecopilot agent agente-ts apk caddy-ui immagine-telefono schermo-web strumenti supervisore web-ts docker-compose.yml .env.esempio personalizzazione.esempio.json
```
La sua chiave pubblica va in `/home/oecopilot/.ssh/authorized_keys`. I secret del repository (`gh secret set <NOME> --repo <proprietario>/<repository>`): `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`, `DEPLOY_KNOWN_HOSTS` (da `ssh-keyscan -t ed25519 <server>`), `ADMIN_EMAIL`, `ADMIN_PASSWORD`, `API_KEY` e `BETTER_AUTH_SECRET` (4.7).

### 4.7 L'interfaccia nel browser

- **`gateway`** (`server/web-ts/`, Next.js e Better Auth, `127.0.0.1:3190`): accesso, pagina, API (`POST /api/<metodo>`) e loghi. Senza sessione le pagine vanno a `/login` e l'API dà 401. Restano aperti `/login`, `/api/auth/*`, `/salute`, `/disponibilita`, `/api/prenota`, `/condividi/...` e i file statici come `/sw.js`.
- **`caddy-ui`**: HTTPS, solo su `127.0.0.1:4445`.
- **`ui-dati/`**: `gateway.db`, cache, loghi e `personalizzazione.json`.

Messa in funzione, poi `UI_HOST=<nome del sito>` in `.env`:
```bash
# dati dell'interfaccia: i container li scrivono come utente 10001, senza privilegi
sudo install -d -o 10001 -g 10001 -m 700 /opt/oe-copilot/ui-dati
# facoltativo: dal modello server/personalizzazione.esempio.json (sul server in /opt/oe-copilot/)
sudo install -o 10001 -g 10001 -m 600 personalizzazione.json /opt/oe-copilot/ui-dati/
```

**Trappola: lo stato sta nel database** (`gateway.db`, tabelle `stato_config` ed `eventi_locali`). `config.json` e `local_events.json` sono copie riscritte a ogni modifica e mai rilette: cambiarle non serve (sezione 7).

**Accesso.** Il proprietario nasce a ogni avvio da `ADMIN_EMAIL` e `ADMIN_PASSWORD`; con una email nuova il vecchio utente va tolto da `gateway.db` (tabelle `session`, `account`, `user`). Il cookie dura 30 giorni, e un cambio di `ADMIN_PASSWORD` chiude tutte le sessioni. Cinque password sbagliate bloccano l'account 15 minuti, per tutti: dietro sslh i client hanno lo stesso indirizzo.

**Secret.** `ADMIN_EMAIL`, `ADMIN_PASSWORD` (almeno 12 caratteri, senza `$`), `API_KEY` e `BETTER_AUTH_SECRET` (almeno 32 caratteri, `openssl rand -hex 32`) stanno nei secret della Action, e compose li legge dall'ambiente prima che da `.env`. Si cambiano con `gh secret set` e un nuovo aggiornamento. Senza la Action stanno in `.env`.

**Trappola: niente `docker compose up` a mano con la Action.** Ricrea l'agente con `API_KEY` vuota, e non parte. Ricrea il gateway senza `BETTER_AUTH_SECRET`, e si ferma con "BETTER_AUTH_SECRET deve essere impostata". Dopo una modifica a `.env` serve un nuovo aggiornamento: `docker restart` non rilegge l'ambiente.

**HTTPS.** Caddy ottiene il certificato da solo. Con la 443 libera bastano le porte `443:443` e `80:80` su `caddy-ui`, lasciando `127.0.0.1:4445:443` per il controllo del deploy. Con sslh sulla 443 si aggiunge a `/etc/sslh.cfg`, sopra la voce che prende tutto il resto:
```
{ name: "tls"; host: "127.0.0.1"; port: "4445"; sni_hostnames: [ "<UI_HOST>", "<MEET_HOST>" ]; },
```
e si riavvia sslh. Il certificato arriva col TLS-ALPN; il primo tentativo HTTP-01 fallisce, ed è previsto.

#### Secondo fattore dell'accesso

`AUTH_2FA` in `.env`: `off` (predefinito, solo password, `/sicurezza` risponde 404), `totp` (dopo la password il codice di un'app di autenticazione o un codice di recupero) o `passkey` (anche le passkey, legate all'indirizzo del sito). Un altro valore ferma il gateway con `AUTH_2FA non valido`. Dopo un nuovo aggiornamento il codice si attiva da *Impostazioni > Generale > Gestisci l'accesso*; i 10 codici di recupero si vedono una volta sola. Cambiare `BETTER_AUTH_SECRET` invalida i codici attivati.

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

- **Gruppi** (*Account > Gruppi*, voce `gruppi_calendari`): un pulsante nella barra lascia accesi solo i calendari del gruppo. Chi ci sta si sceglie nella pagina di ogni casella o calendario.
- **Account uniti** (voce `account_aliases`): più account sotto un nome solo. Vince su `nomi_per_indirizzo` e `nomi_per_dominio`.
- **Secondario** (voce `secondari_scelti`): la voce va sotto *Altri calendari*. Vince su `interfaccia.calendari_secondari`.
- **Disponibilità pubblica**: `https://<UI_HOST>/disponibilita`, senza accesso. Solo blocchi "Occupato", da tutti i calendari, anche spenti, senza giorni interi né finestre da 24 ore in su (un change di ServiceNow di settimane, `impegni.ts`), a meno che siano blocchi voluti (sotto). `?gruppo=<nome>` la limita a un gruppo. Con `MEET_HOST=<altro nome>` in `.env` (stesso IP, e il nome in `sni_hostnames` di sslh) la pagina sta anche alla radice di quel nome, `https://<MEET_HOST>/?gruppo=<nome>`, e lì il resto del sito non c'è: ogni altro indirizzo torna alla radice.
- **Prenotazione di un meeting**: sulla pagina della disponibilità un clic su uno spazio libero apre il riquadro per prenotare (giorno, dalle, alle a quarti d'ora, nome, email, azienda, argomento). Si prenota dal lunedì al venerdì, dalle 9 alle 18 di Roma, con due ore di preavviso e fino a 60 giorni avanti (`lib/prenota-regole.ts`), solo dove i calendari danno libero. Il server (`POST /api/prenota`, `lib/prenota.ts`) ricontrolla tutto, una prenotazione alla volta, al più 6 in un'ora. L'evento entra subito come evento personale che blocca il tempo: con `?gruppo=` per quel gruppo, senza per tutti i gruppi. Nelle note ci sono email, azienda e argomento, e arriva una notifica push. Chi prenota scarica l'invito `.ics`; email il server non ne manda, e il link per collegarsi si manda a mano.
- **Eliminati** (voce `eventi_eliminati`): *Elimina* sulla scheda di una riunione delle caselle o dei link la toglie solo qui, perché il telefono si legge e basta. Esce dalla pagina, dalla disponibilità, dalla condivisione e dai promemoria; torna con *Annulla* nell'avviso o con *Ripristina* in *Account > Eliminati*, per 7 giorni (`eliminato_il`; le voci senza data contano dalla fine dell'evento); ogni voce dice quando scade. Dopo resta eliminata, fuori dall'elenco: la voce in configurazione resta, perché è lei a tenere fuori l'evento. La riconosce titolo (senza `FW:`/`RE:`), inizio e fine: spostata a un'altra ora, ricompare. Un evento personale invece si cancella davvero.
- **Blocchi di tempo**: un giorno intero o un impegno lungo rende occupati nella disponibilità se è voluto. Dall'app: *Nuovo evento* con *Blocca il tempo* (acceso di partenza; spento è solo una nota nel tuo calendario). Da Outlook o Google: *Mostra come: Occupato* (l'agente legge `availability` di `calendar.db`; un giorno intero è di solito *Libero*, i change di ServiceNow *Provvisorio*). Un giorno intero bloccato occupa da mezzanotte a mezzanotte, e quello delle caselle si vede anche nella pagina. Un evento personale fatto prima di questa scelta resta come prima.
- **A chi**: un evento personale (*Nuovo evento* o clic sulla griglia) può essere per uno o più gruppi dei calendari, a caselle. Nella vista di un gruppo esce con le sue riunioni, anche se gli *Eventi personali* sono spenti; in quella di tutti esce una volta sola; conta nella disponibilità e nei link condivisi del gruppo. Senza gruppi resta com'era: solo con gli *Eventi personali* accesi, e fuori dalla disponibilità dei gruppi.
- **Riunioni lunghe**: in Giorno, 3 giorni e Settimana una riunione da 4 ore in su sta nella riga in cima alle colonne, con i giorni interi (`inAlto` in `server/web-ts/src/lib/oe/logic.ts`). Non nella disponibilità pubblica, che non ha la riga in cima: lì ogni blocco "Occupato" resta alle sue ore.

#### Loghi e calendari da link

**Loghi.** Voce `interfaccia.loghi` di `personalizzazione.json`: un file in `ui-dati/logos/` (`"logos/alfa.png"`) o un indirizzo, che il gateway scarica e rinnova ogni 7 giorni. Il pulsante *Logo* nella pagina di una casella o di un calendario (*Account*) sceglie un logo per voce, e vince sul file. Il browser non carica mai loghi da altri siti.

**Calendari da link.** *Account > Da link e personali*. Si rileggono ogni 30 minuti; una risposta 404 o 410 toglie la copia. Un calendario Google entra col suo link iCal segreto, che vale come una password e resta sul server; se l'amministratore l'ha disattivato, l'account si aggiunge nel telefono (4.4).

**Trappola: solo siti pubblici.** Il gateway scarica (`server/web-ts/src/lib/core/rete-sicura.ts`) solo `http` o `https` sulle porte 80 e 443, da nomi con soli indirizzi pubblici: niente loopback, reti private, link-local (`169.254.169.254`) o CGNAT, anche dopo un reindirizzamento. Un logo: al più 512 KB e 4096 pixel per lato, SVG senza script. Un calendario: al più 8 MiB, che comincia con `BEGIN:VCALENDAR`, con Content-Type assente o `text/calendar`, `text/plain`, `application/octet-stream` (`application/ics` è rifiutato). Vicino agli 8 MiB può superare gli 8.000.000 byte che l'agente accetta.

#### Personalizzazione

Nomi delle aziende, ordine dei calendari, sigle, loghi e schede del Timesheet stanno in `ui-dati/personalizzazione.json`, fuori dal repository; `server/personalizzazione.esempio.json` ha un esempio di ogni voce. Senza il file ogni calendario prende il nome dal dominio della sua casella. Il gateway rilegge il file quando cambia.

| Voce | A cosa serve |
|------|--------------|
| `nomi_per_indirizzo` | Nome per una casella precisa |
| `nomi_per_dominio` | Nome per tutte le caselle di un dominio |
| `domini_aziendali` | Un invito organizzato da questi domini prende il nome dell'azienda, anche nel calendario di un'altra casella |
| `caselle_di_colleghi` | Caselle di altre persone aperte in Outlook: i loro eventi restano loro |
| `ordine_calendari` | Ordine nella barra laterale |
| `azienda_per_cartella` | Cartelle calendario di un'azienda, riconosciute dal nome |
| `account_nascosti` | Caselle da non mostrare mai |
| `mostra_tutto_il_giorno` | `true` mostra gli eventi di un giorno intero delle caselle |
| `aziende_inoltrate` | Aziende le cui riunioni arrivano copiate in un'altra casella (sotto) |
| `interfaccia` | Sigle, loghi, calendari secondari, calendari senza "Apri in Teams", schede Timesheet (`schede_timesheet`: un clic apre `url`) |

**Aziende inoltrate.** Un flusso (per esempio Power Automate) può copiare le riunioni di un account aperto solo nel browser nel calendario di un'altra casella, con un prefisso nel titolo come `Copia: Riunione`. In `aziende_inoltrate` vanno la casella che le riceve (`via`), il prefisso (`prefisso`, espressione regolare) e, facoltativo, il `colore`. Le copie si mostrano come l'azienda d'origine, senza prefisso. Il pallino segue la casella che le ospita: un flusso fermo non si vede.

### 4.8 Lo schermo del telefono nel browser

Il servizio `schermo` (`server/schermo-web/`) mostra e comanda il telefono: pulsante *Telefono*, pagina `/telefono/`. È una copia potata di [ws-scrcpy](https://github.com/NetrisTV/ws-scrcpy) (commit `cd6cea6`, licenza MIT in `server/schermo-web/LICENSE`): la prima commit della cartella è la copia di upstream, le altre mostrano ogni modifica. Sul telefono gira il fork WebSocket di scrcpy 1.19 (`1.19-ws8`, licenza Apache-2.0), in `vendor/Genymobile/scrcpy/` con la sua impronta; il server di scrcpy 5 non costa meno e non parla il protocollo della pagina.
- **Trappola: nessuna password.** Il servizio non ha porte pubblicate né password, e adb non chiede conferme (`ro.adb.secure=0`). L'unico cancello è Caddy, che prima di passare `/telefono/*` chiede `/api/verifica-sessione` al gateway (`forward_auth`).
- Rispetto a upstream, `proxy-adb` raggiunge solo `tcp:8886`, il server di scrcpy, che ascolta solo sul localhost del telefono; la pagina prende il flusso solo dal proprio sito.
- La tastiera arriva come caratteri, le accentate come accento morto più lettera, Ctrl+V incolla; €, £ ed emoji no. Il pannello del pulsante `?` (`GoogHelpBox.ts`) si cambia insieme a `KeyText.ts` e alla barra.
- Una scheda aperta riscrive ogni 10 secondi `stato/schermo/PAUSA` e ferma il supervisore; dopo 60 secondi il file non conta più.
- Il server scrcpy resta nel telefono anche a servizio fermo: *Kill server* nella pagina `/telefono/` lo ferma.

### 4.9 I promemoria

Si scelgono nel sito, *Impostazioni > Promemoria*, e stanno sul server (voce `promemoria`, metodi `get_promemoria` e `set_promemoria`). L'app per Windows le rilegge ogni minuto, quella per Android ogni 15 minuti e quando passa in secondo piano.
- **Quando**: 1 ora, 30, 15, 10 o 5 minuti prima, *All'inizio*, e altri minuti in un campo. Al più 8 valori da 0 a 1440 (0 è all'inizio); di norma 10.
- **Dove**: nell'app per Windows *Notifica di Windows* (il toast, una per riunione con il logo dell'azienda, che sparisce all'inizio) e *Avviso a comparsa* (in basso a sinistra, visibile anche con *Non disturbare*). In un browser normale *Notifiche del browser*, che iscrive o toglie quel browser. L'app per Android avvisa sempre, anche chiusa.
- **Contenuto**, per tutti i canali: *Completo* (titolo e luogo) o *Solo ora e azienda* (l'avviso a comparsa mostra logo e ora).
- **Suono**: solo l'app per Windows, tra i suoni di sistema. Lo fa l'avviso a comparsa; ad avviso spento, la notifica.

Le voci degli avvisi le calcola il nucleo delle app (`app/src-tauri/src/promemoria.rs`): su Windows avvisa lo script della pagina, finché l'app è in esecuzione; su Android un lavoro in background programma un allarme per avviso, che parte anche ad app chiusa (`app/README.md`). Le notifiche del browser le manda il server, anche a pagina chiusa, con un giro ogni `PROMEMORIA_INTERVALLO_S` secondi (di norma 30). Un avviso per riunione e anticipo; i giorni interi e le riunioni eliminate dalla pagina non avvisano. Gli interruttori dei calendari nella barra laterale filtrano solo la vista: una riunione di un calendario spento avvisa lo stesso (le app chiedono `get_events` con un terzo argomento, `true`). Una cancellazione fatta in Outlook arriva a telefono sbloccato: prima la riunione può avvisare.

**Notifiche del browser.** Il testo passa cifrato (RFC 8291) dal servizio di notifiche del browser. Le WebView delle app non hanno notifiche web, quindi nell'app questo interruttore non c'è. Servono le chiavi VAPID:
   ```bash
   node -e "const c=require('crypto');const j=c.generateKeyPairSync('ec',{namedCurve:'P-256'}).privateKey.export({format:'jwk'});console.log('VAPID_PUBLIC_KEY='+Buffer.concat([Buffer.from([4]),Buffer.from(j.x,'base64url'),Buffer.from(j.y,'base64url')]).toString('base64url'));console.log('VAPID_PRIVATE_KEY='+j.d)"
   ```
Le due righe vanno nel `.env` della VM con `VAPID_SUBJECT=mailto:<indirizzo>` (o `https://<sito>`), poi un deploy con la Action `Server`. Senza i tre valori, o in un browser senza notifiche, l'interruttore non compare. La chiave privata e gli indirizzi delle iscrizioni (tabella `push_iscrizioni`, al più 20 browser) sono segreti e non finiscono nei log. Prove: `cd server/web-ts && npx vitest run test/promemoria`.

### 4.10 Condividere il calendario con un altro OE Copilot

Un link ICS per persona, `https://<UI_HOST>/condividi/<testo segreto>.ics`. L'altro OE Copilot lo aggiunge con *Account > Da link e personali* e lo rilegge ogni 30 minuti.
- **Gestione**: *Impostazioni > Condivisione > Gestisci i link*. Per ogni link: livello, cosa condividere, giorni indietro (0-30, di norma 7) e avanti (1-365, di norma 90). Al più 50 link, nella tabella `condivisioni`.
- **Livelli**: *Occupato* dà solo gli orari; *Con titoli* aggiunge titolo e luogo, mai descrizione, partecipanti o link per collegarsi.
- **Cosa esce**: i calendari accesi secondo `disabled_calendars` salvato sul server, o un gruppo. Mai quelli ricevuti da altri (link ICS, festività, altre condivisioni). Gli interruttori della pagina non cambiano più quell'elenco (4.11): resta quello di prima.
- **Revoca** e **Cambia indirizzo** valgono subito: l'indirizzo vecchio dà 404, e chi lo legge toglie la copia.
- **Trappola: 503, mai vuoto.** Agente fermo, casella in errore, gruppo sparito o più di 5000 eventi danno 503 con `Retry-After`: un calendario vuoto vorrebbe dire "libero". Il motivo sta nel log del gateway.
- **Un link è una chiave.** Il gateway non lo scrive nei log, e il filtro del Caddyfile lo toglie da quelli di Caddy (`/condividi/REDATTO`). In `condivisioni` sta in chiaro: una copia di `ui-dati/` va protetta.

### 4.11 Impostazioni del dispositivo e uso senza rete

- **Per dispositivo.** Tema, zoom, vista, colori dinamici, eventi di tutto il giorno nascosti (l'interruttore sotto il mese piccolo: toglie i giorni interi, anche Occupato, e gli eventi che durano un giorno o più), calendari secondari, intervallo di lettura e calendari accesi stanno nel `localStorage` di ogni browser e di ogni app (`oe.impostazioni-dispositivo`, `VOCI_DEL_DISPOSITIVO` in `server/web-ts/src/lib/oe/logic.ts`). Un dispositivo nuovo parte dalla config del server. Nomi, gruppi, link ICS, loghi e promemoria restano sul server. La pagina manda i suoi calendari spenti come quarto argomento di `get_events`.
- **Senza rete.** `public/sw.js` tiene la pagina `/` e i suoi file, e l'ultima risposta di `get_config`, `get_personalizzazione`, `get_calendars`, `get_events` e `get_loghi`. Senza rete la pagina si apre da lì, con la fascia "Senza rete. Dati aggiornati alle HH:MM", e rilegge appena la rete torna. Le scritture senza rete falliscono come prima.
- **Le app.** La pagina di avvio porta al sito anche quando il server non risponde: se il sito non ha ancora una copia (mai aperto con questa versione) la finestra torna alla pagina di avvio, che riprova.
- **Trappola: i dati restano nel dispositivo.** Gli eventi letti stanno in Cache Storage (`oe-1-dati`) finché non si esce dall'account (*Esci*), che li cancella.

### 4.12 MCP per le email del telefono (`/mcp`)

`https://<UI_HOST>/mcp` (Streamable HTTP, JSON-RPC `2025-06-18`) espone ai client AI tutte le caselle di posta Outlook collegate sul telefono virtuale, senza passare da registrazioni Azure o consensi amministratore.
- **Autenticazione**: `Authorization: Bearer <MCP_TOKEN>` (se `MCP_TOKEN` è impostato nell'ambiente del gateway) oppure `Authorization: Bearer <API_KEY>`. Le richieste con intestazione `Origin` (provenienti da un browser) sono rifiutate con 403.
- **Come raccoglie i token**: `prepara_telefono.sh` estrae in `stato/chiave-outlook.bin` (permessi `0600`, utente `OUTLOOK_UID`) la chiave AES-256-GCM (`HxCoreSecureStringAES256Gcm`) dal keystore di Android (`persistent.sqlite`). A ogni giro del rilevatore, `supervisore` decifra i token OAuth da `acompliAcct.db` (`tokenStore` e `mailAccounts.directToken`) e scrive `stato/token-outlook.json`, che l'agente serve al gateway su `GET /api/token-posta`.

Registrazione in Claude Code:
```bash
claude mcp add --scope user --transport http oe-copilot-mail https://<UI_HOST>/mcp --header "Authorization: Bearer <API_KEY>"
```

| Tool | Cosa fa |
|---|---|
| `list_accounts` | Caselle Outlook attive sul telefono, scadenza del token e stato di sincronizzazione |
| `list_folders` | Cartelle e sottocartelle della casella con conteggio messaggi totali e non letti |
| `list_emails` | Messaggi di una cartella (`Inbox`, `SentItems`, `Drafts`, `Archive`, `DeletedItems`, cartella specifica o `all`), dal più recente, con filtri `query`, `from_address`, `unread_only`, `has_attachments`, `flagged_only`, `start_date`, `end_date` |
| `read_email` | Singola email completa: `From`, `To`, `Cc`, `Bcc`, corpo HTML convertito in Markdown (tabelle e link SafeLinks inclusi) ed elenco allegati |
| `read_thread` | Intero thread di conversazione (`thread_id` = `ConversationId` o qualsiasi `message_id` del thread) ordinato dal più vecchio al più recente, con elenco partecipanti, corpi Markdown e allegati |
| `search_emails` | Ricerca full-text su tutte le cartelle (o una cartella scelta) con filtri per mittente, date, non lette e allegati |
| `send_email` | Nuova email, risposta (`reply_to_id`), risposta a tutti (`reply_all`), inoltro (`forward_to_id`) o salvataggio in bozza (`save_as_draft`). Supporta HTML ricco (`is_html` o tag HTML nel `body`) anche nelle risposte, destinatari selettivi (`to`, `cc`, `replace_cc`, `exclude_recipients`) e allegati (`attachments`) |
| `forward_email` | Inoltra un messaggio esistente mantenendo allegati originali e storico citato, con commento HTML/testo, `to`, `cc`, `bcc` e allegati aggiuntivi |
| `get_attachment` | Scarica gli allegati di un messaggio: immagini restituite inline, file di testo come testo, binari come risorse incorporate, ed email/eventi allegati (`ItemAttachment`) espansi in Markdown |
| `manage_email` | Esegue `mark_read`, `mark_unread`, `flag`, `unflag`, `archive`, `delete` o `move` (verso `destination_folder`) su uno o più `message_id` |

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

`server/agente-ts/` legge in sola lettura `agent/fonti.json` e il `calendar.db` del telefono, da una copia aperta con `PRAGMA query_only = ON` (Android tiene aperto l'originale). Gira come utente 10001, in sola lettura e senza capacità. L'API, con `Authorization: Bearer <API_KEY>` e una cache di 15 secondi: `GET /api/eventi?inizio=...&fine=...`, `GET /api/calendari`, `GET /api/salute`, `POST /api/ics`. `src/entita.ts` e `src/windows.ts` sono tabelle generate: non si correggono a mano.

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
- **Prove della Action** (job *Controlli*, solo per `server/` e il workflow): sintassi, `docker compose config` (l'elenco dei servizi è scritto nel workflow), Caddyfile, tipi, `npm test` e immagini dei servizi, poi *Gateway, accesso e cancello* sui container veri.
- **Prove a mano**: in `server/agente-ts`, `server/supervisore`, `server/schermo-web` e `server/web-ts` `npm ci --ignore-scripts`, `npm run typecheck` e `npm test`; in `schermo-web`, prima di `npm test`, anche `npm run dist` e `npm run smoke`, che vanno senza telefono. `app/` ha le sue ([`app/README.md`](app/README.md#prove)). Prima di un push, `python server/strumenti/controlla_dati_personali.py <personalizzazione.json>` fallisce se trova i dati della personalizzazione nei file del repository.
- **Risposte registrate** (`server/strumenti/golden/`, `server/supervisore/test/golden/`): dati fissi, mai rigenerati. Una differenza è un comportamento cambiato. I casi nuovi del rilevatore vanno in `server/supervisore/test/rilevatore.test.ts`: il confronto vuole i nomi dei casi registrati.
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
| *Il server dei calendari non risponde* | `docker logs oe-copilot-agent` e `docker logs oe-copilot-gateway` | Il suggerimento del pallino giallo o rosso dice cosa è fermo |
| Un calendario nuovo non compare | La chiamata all'agente di 4.5, su `/api/calendari` | finestra *Account*, freccia in alto a sinistra (rilegge le caselle); in Outlook sul telefono la spunta e *Sincronizza calendari* (4.4); la casella in `agent/fonti.json`, poi `sudo docker compose restart agent` |
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
