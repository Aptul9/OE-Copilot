# OE Copilot — guida completa (PC + server)

Guida per installare OE Copilot da zero su un **PC Windows** e, se si vuole, su un **server Linux**
(la "VM") che legge i calendari al posto del PC. Spiega anche *perché* è fatto così, perché le
scelte non sono ovvie e senza il ragionamento è facile rifarle male.

> In breve: l'app unisce in un calendario unico tutte le caselle (aziendali e personali), con un
> colore per azienda e i promemoria. Può leggere le caselle in due modi:
> - **modalità PC** — legge direttamente Outlook desktop già installato sul PC (nessun server);
> - **modalità server** — le caselle le legge un agente su un server e il PC gliele chiede
>   attraverso un tunnel SSH. Così sul PC non serve tenere Outlook aperto né far "girare" i
>   calendari: gli accessi stanno sul server.
>
> La modalità PC basta e avanza se usi un solo computer. La modalità server serve se non vuoi (o
> non puoi) far leggere i calendari al tuo PC, o se vuoi che i promemoria funzionino anche a PC
> spento.

Indice:
1. [I ragionamenti (perché è fatto così)](#1-i-ragionamenti-perché-è-fatto-così)
2. [Architettura](#2-architettura)
3. [Installazione sul PC](#3-installazione-sul-pc)
4. [Installazione sul server (modalità server)](#4-installazione-sul-server-modalità-server)
5. [Collegare il PC al server](#5-collegare-il-pc-al-server)
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
firma di Google). [DAVx⁵](https://www.davx5.com/) (CalDAV) resta supportato dall'agente, ma il suo
login Google richiede un client OAuth proprio: quello della build OSE non è più accettato da Google
(`Error 401: invalid_client`).

**Perché un servizio che forza la sincronizzazione.** Due vincoli di Android:
- la sincronizzazione periodica ha un **minimo di 15 minuti**;
- Outlook, senza i servizi Google (niente notifiche push), **scarica le novità dal server solo
  quando la sua schermata principale è in primo piano**: fermo nelle impostazioni non riceve nulla.

Il servizio `oe-copilot-sincronizza` gira sul server e ogni 30 s forza i lavori di sincronizzazione,
tiene Outlook in primo piano e spinge DAVx⁵. Così l'aggiornamento è di ~20–30 s invece di 15 min.

**Perché un tunnel SSH (e non un'API esposta su internet).** L'agente ascolta **solo sul localhost
del server**. Il PC lo raggiunge con un tunnel SSH che l'app apre da sola: su internet non è esposto
nulla, l'accesso passa dalla tua chiave SSH, più una chiave dell'API. Nessuna porta aperta, nessun
certificato da gestire.

**Perché il rilevatore degli accessi scaduti.** Se un login sul telefono scade (password cambiata,
sessione revocata, MFA), il calendario Android resta com'era e la lettura **riesce lo stesso**: il
pallino resterebbe verde su dati fermi (un "falso verde"). Il rilevatore se ne accorge e mette il
pallino rosso. Vedi la [sezione 6](#6-il-rilevatore-degli-accessi-scaduti).

---

## 2. Architettura

```
                 PC Windows                                   Server Linux
 ┌─────────────────────────────────┐          ┌──────────────────────────────────────────┐
 │  OE Copilot (app Python)         │          │  Docker:                                 │
 │   interfaccia (FullCalendar)     │          │   ┌───────────────┐   ┌────────────────┐ │
 │   eventi.py  ──►  agente_remoto ─┼─ tunnel ─┼─► │ agent (HTTP)  │◄──┤ telefono       │ │
 │                                  │   SSH    │   │ 127.0.0.1:3180│   │ (redroid)      │ │
 │   (promemoria Windows/Telegram)  │          │   └──────┬────────┘   │ Outlook+Google │ │
 └─────────────────────────────────┘          │          │ legge      └───────┬────────┘ │
                                               │   calendar.db (sola lettura) │          │
                                               │   servizio oe-copilot-sincronizza ◄──────┘
                                               │   (forza sync ogni 30s, rileva accessi)   │
                                               └──────────────────────────────────────────┘
```

- **telefono** = container redroid con l'app Outlook (caselle Microsoft) e i servizi Google
  (calendari Google). Le app copiano gli appuntamenti nel calendario di sistema di Android.
- **agent** = piccolo server HTTP che legge quel calendario e lo espone con un'API a chiave, in
  ascolto solo su `127.0.0.1:3180` del server.
- **servizio** = tiene aggiornato il telefono e scrive i "segni di vita" (`battito.json`,
  `accessi.json`).
- **app sul PC** = apre il tunnel SSH e chiede gli eventi all'agent; per il resto funziona come in
  modalità PC (griglia, colori, promemoria).
- **interfaccia nel browser** (facoltativa, 4.7) = la stessa pagina servita dal server dietro
  HTTPS e password; chiede gli eventi all'agent sulla rete di Docker.

Il codice del server sta nella cartella **`server/`** del repository.

---

## 3. Installazione sul PC

### Requisiti
- Windows 10/11.
- **Outlook desktop classico** installato e configurato (serve per la modalità PC e per il pulsante
  "Apri in Outlook"; non il "Nuovo Outlook").
- **Python 3.12** (da python.org, oppure con [scoop](https://scoop.sh):
  `scoop bucket add versions` e `scoop install python312`).
- Runtime **Microsoft Edge WebView2** (di norma già presente su Windows 11).
- Connessione a internet al primo avvio (FullCalendar arriva da una CDN).

### Passi
Dalla cartella dell'app, in un prompt dei comandi:

```bat
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python strumenti\applica_patch_pywebview.py
```

Se Python 3.12 arriva da scoop il lanciatore `py` non lo vede: al posto di `py -3.12` usa
`%USERPROFILE%\scoop\apps\python312\current\python.exe`.

L'ultimo comando applica a pywebview due correzioni che servono all'app (icona nella barra delle
applicazioni, finestra senza cornice). Va ripetuto se si reinstalla pywebview.

Poi copia `personalizzazione.esempio.json` in `personalizzazione.json` e mettici i tuoi dati
(nomi aziende, ordine calendari, sigle, schede Timesheet…). Tutte le voci sono facoltative: senza
il file l'app funziona e ogni calendario prende il nome dal dominio della casella. Dettaglio di ogni
voce nel [README](README.md).

### Avvio
- Prima volta: doppio clic su **`start.bat`** (gli errori finiscono in `debug.log`/`crash.log`).
- Uso quotidiano: **`Calendar Hub (silenzioso).vbs`** (senza finestre di prompt).
- Controllo di salute: `.venv\Scripts\python verifica.py` (con `--avvio` lancia anche l'app).
  Se Outlook era chiuso lo avvia e quel giro segna rosso "Outlook risponde": rilancialo dopo
  una ventina di secondi.

A questo punto, in **modalità PC**, hai finito. Se vuoi la modalità server, continua.

---

## 4. Installazione sul server (modalità server)

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
`account` è l'indirizzo esatto come lo vedi in Outlook/DAVx⁵ sul telefono; `nome` è l'etichetta che
vuoi mostrare; `id` è un identificativo qualsiasi ma univoco.

Di una casella Outlook l'agente legge solo il calendario principale. Per leggerne altri (i
calendari che altre caselle o altre aziende condividono con quella, "People's calendars") elencali
per nome nella voce, **principale compreso**:
```json
{"tipo": "android", "id": "android::mario.rossi@azienda.example", "nome": "Azienda",
 "account": "mario.rossi@azienda.example", "calendari": ["Calendar", "Beta", "Gamma"]}
```
Il nome è quello che si vede in Outlook. Ogni evento porta con sé il nome del suo calendario, e sul
PC `azienda_per_cartella` lo usa per mostrarlo come l'azienda giusta, per esempio
`"azienda_per_cartella": {"· Beta": "Beta"}`. Per dare a quell'azienda una voce, un colore e un
pallino propri serve anche una voce in `aziende_inoltrate` con `via` uguale alla casella che la
ospita.

### 4.3 Avviare lo stack
```bash
sudo docker compose up -d        # avvia 'telefono' (redroid) e 'agent'
sudo docker ps                   # devono essere "Up" entrambi
```
Il primo avvio scarica le immagini. Se il telefono non parte, quasi sempre manca il modulo binder
(vedi 4.1).

### 4.4 Configurare il telefono virtuale
**Impostazioni e app.** Fuso orario, lingua, schermo sempre acceso, le app (Outlook, DAVx⁵, Firefox), niente
risparmio batteria per Outlook e DAVx⁵ e il permesso sul calendario del sincronizzatore di Google li
mette lo script, senza toccare lo schermo:
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
gh release download apk --repo <proprietario>/OE-Copilot --pattern '*.apk' --dir /opt/oe-copilot/apk
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

**Schermo.** Il telefono si vede dal **tuo PC** con [scrcpy](https://github.com/Genymobile/scrcpy)
(`scoop install scrcpy`, oppure lo zip dal sito: contiene anche `adb`). Dalla cartella dell'app:
```bat
.venv\Scripts\python strumenti\schermo_telefono.py <utente@server>
```
Lo script mette in pausa il servizio che altrimenti ti "ruberebbe" lo schermo (file `PAUSA`), apre
un tunnel SSH verso la porta adb del telefono (la `5555`, esposta solo sul localhost del server),
collega adb e apre la finestra. Quando la chiudi toglie la pausa e chiude il tunnel. Se
`agente_remoto` è già nella personalizzazione l'indirizzo si può omettere.

<details><summary>Gli stessi passi a mano</summary>

```bat
ssh <utente@server> "sudo touch /opt/oe-copilot/telefono/PAUSA"
ssh -N -L 15555:127.0.0.1:5555 <utente@server>
rem in un altro prompt:
adb connect 127.0.0.1:15555
scrcpy -s 127.0.0.1:15555
rem alla fine:
ssh <utente@server> "sudo rm /opt/oe-copilot/telefono/PAUSA"
```
</details>

**Login**, nella finestra del telefono:
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

> I login (password e MFA) li fai **tu**, dalla finestra del telefono.

### 4.5 Il servizio di sincronizzazione
Installa la unit systemd (tiene aggiornato il telefono ogni 30 s e scrive i segni di vita):
```bash
sudo cp /opt/oe-copilot/server/strumenti/oe-copilot-sincronizza.service /etc/systemd/system/
# (se hai copiato server/ in /opt/oe-copilot, lo script è già in /opt/oe-copilot/strumenti/)
sudo systemctl daemon-reload
sudo systemctl enable --now oe-copilot-sincronizza
systemctl status oe-copilot-sincronizza        # deve essere "active (running)"
```
Cosa fa, a ogni giro: forza i lavori di sincronizzazione del calendario per ogni casella, spinge
DAVx⁵, **riporta Outlook sulla schermata principale** (altrimenti non scarica), e scrive
`stato/battito.json`. Ogni ~5 minuti lancia anche il **rilevatore degli accessi** (sezione 6).

Verifica che l'agente risponda (dal server):
```bash
curl -s -H "Authorization: Bearer $(sudo grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2)" \
  http://127.0.0.1:3180/api/calendari | python3 -m json.tool | head
```

### 4.6 Aggiornamento automatico (facoltativo)
La GitHub Action `.github/workflows/server.yml` porta `server/` sul server a ogni push su `main`
che lo tocca: controlla la sintassi, copia il codice con rsync e lancia
`strumenti/aggiorna_server.sh`, che rimette a posto impostazioni e app del telefono
(`prepara_telefono.sh`, con l'APK di Outlook preso dalla release `apk`), ricostruisce l'agente, riavvia il servizio e verifica che
l'agente risponda e che il battito sia fresco. Se la verifica fallisce rimette la versione di
prima. `.env`, `agent/fonti.json`, `telefono/`, `stato/` e `ui-dati/` non vengono mai toccati. Il
telefono si riavvia solo se cambia la sua parte di `docker-compose.yml`. Se in `.env` c'è
`UI_HOST`, a fine aggiornamento parte anche l'interfaccia nel browser (4.7): un suo guasto rende
rossa la Action ma non riporta indietro l'agente né il telefono.

Sul server serve, una volta sola, un utente dedicato che entra solo con una chiave SSH:
```bash
sudo useradd -m -s /bin/bash -G docker oecopilot
sudo chown oecopilot:oecopilot /opt/oe-copilot /opt/oe-copilot/.env
sudo chown -R oecopilot:oecopilot /opt/oe-copilot/agent /opt/oe-copilot/strumenti /opt/oe-copilot/docker-compose.yml
# la unit punta al file nel repository: così si aggiorna anche lei
sudo ln -sf /opt/oe-copilot/strumenti/oe-copilot-sincronizza.service /etc/systemd/system/
echo 'oecopilot ALL=(root) NOPASSWD: /usr/bin/systemctl daemon-reload, /usr/bin/systemctl restart oe-copilot-sincronizza' \
  | sudo tee /etc/sudoers.d/oe-copilot >/dev/null && sudo chmod 440 /etc/sudoers.d/oe-copilot
```
La chiave pubblica della Action va in `/home/oecopilot/.ssh/authorized_keys`. Nel repository
servono quattro secret: `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY` (la chiave privata) e
`DEPLOY_KNOWN_HOSTS` (la riga di `ssh-keyscan -t ed25519 <server>`). Senza i secret
l'aggiornamento fallisce al collegamento SSH: chi non ha un server disattiva la Action.

### 4.7 L'interfaccia nel browser (facoltativa)
La stessa pagina e la stessa logica dell'app per Windows, servite dal server a un browser
qualunque, con HTTPS e password: stesse caselle, stessi colori, pallini e filtri. Gli interruttori
(calendari, aziende, alias, calendari da link ICS, eventi personali) restano sul server. Non ci
sono i promemoria (restano all'app per Windows), i controlli della finestra, l'avvio con Windows
e *Apri in Outlook*; i link delle riunioni si aprono nel browser.

Come è fatta, in `server/docker-compose.yml`:
- **`ui`**: un piccolo server Python (`server/ui/server.py`) che serve `web/` e chiama la classe
  `Api` di `app.py`, la stessa che la finestra di Windows chiama con pywebview. I moduli e la
  pagina dell'app li copia nell'immagine `strumenti/prepara_ui.sh` (la Action lo lancia da sola):
  il codice resta uno solo. Legge le caselle dall'agente sulla rete di compose
  (`http://agent:8180`), senza tunnel. Non è pubblicato su nessuna porta e non ha password: ci
  arriva solo `caddy-ui`.
- **`caddy-ui`**: Caddy con HTTPS e password (basic auth), solo su `127.0.0.1:4445`. Indirizzo,
  utente e hash della password arrivano da `.env`, non dal repository.
- **`ui-dati/`**: configurazione, interruttori, cache e `personalizzazione.json` (nomi, sigle e
  colori delle aziende: stesso file e stessa forma dell'app sul PC). Sta solo sul server.

Messa in funzione, una volta sola, sul server:
```bash
# dati dell'interfaccia: il container li scrive come utente 10001, senza privilegi
sudo install -d -o 10001 -g 10001 -m 700 /opt/oe-copilot/ui-dati
sudo install -o 10001 -g 10001 -m 600 personalizzazione.json /opt/oe-copilot/ui-dati/
# password: l'hash bcrypt va in .env tra apici singoli (i $ non sono variabili)
read -rs PW && printf '%s' "$PW" | docker run --rm -i caddy:2.11.4-alpine caddy hash-password
```
In `.env` (modello in `.env.esempio`): `UI_HOST=<nome del sito>`, `UI_USER=<utente>`,
`UI_HASH='<hash>'`. Il prossimo aggiornamento costruisce e avvia `ui` e `caddy-ui`.

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

Prova: senza credenziali `curl -I https://<UI_HOST>/` risponde 401 con `WWW-Authenticate: Basic`;
con `-u utente` risponde 200.

**Personalizzazione.** Senza `personalizzazione.json` in `ui-dati/` le aziende prendono il nome dal
dominio della casella e i colori dalla tavolozza. Per averle come sul PC si copia lo stesso file e
si riavvia il servizio (`docker restart oe-copilot-ui`). I colori già assegnati sul PC stanno in
`config.json` (`account_colors`): copiare anche quella voce in `ui-dati/config.json` dà gli stessi.

---

## 5. Collegare il PC al server

1. **Chiave SSH**: il server dev'essere raggiungibile da `ssh` **senza password** (chiave SSH).
   Comodo dare un alias in `~/.ssh/config` del PC, es.:
   ```
   Host oecopilot
       HostName  198.51.100.10
       User      mario
       IdentityFile ~/.ssh/id_ed25519
   ```
   Prova: `ssh oecopilot hostname` deve entrare senza chiedere nulla. Provalo dal prompt dei
   comandi e non da Git Bash: l'app usa l'OpenSSH di Windows, che rifiuta una chiave leggibile da
   altri utenti ("UNPROTECTED PRIVATE KEY FILE", nell'app "Permission denied"), mentre l'ssh di
   Git la accetta. Una chiave in una cartella sincronizzata (OneDrive) eredita spesso permessi in
   più: tienine una copia in `%USERPROFILE%\.ssh` e sistemala con
   `icacls <chiave> /inheritance:r /grant:r "%USERNAME%:R"`.

2. **Chiave dell'API**, salvata cifrata per il tuo utente Windows (non passa mai dallo schermo):
   ```bat
   ssh oecopilot "grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2" | .venv\Scripts\python agente_remoto.py imposta-chiave
   ```

3. In **`personalizzazione.json`** (sul PC) aggiungi la voce `agente_remoto` e togli dai
   `calendari_google` gli account che ora legge il server:
   ```json
   "agente_remoto": {"ssh": "oecopilot", "porta_remota": 3180, "porta_locale": 31800}
   ```
   Con la voce `ssh` presente l'app legge **solo** dal server e non più da Outlook del PC:
   aggiungila quando sul telefono i login sono fatti e `fonti.json` è compilato, altrimenti il
   calendario resta vuoto. Per provare il collegamento prima, senza toccare la personalizzazione
   vera: metti la voce in un file a parte e lancia la prova con
   `set OE_PERSONALIZZAZIONE=<quel file>`.
   Riavvia l'app. Prova dal prompt: `.venv\Scripts\python agente_remoto.py prova` deve elencare le
   caselle.

Per **tornare alla modalità PC** basta togliere la voce `ssh` da `agente_remoto` (o rimettere il
file `personalizzazione.json` di prima) e riavviare.

Cosa cambia nell'app in modalità server: il pallino di ogni azienda dice anche se sul server
l'aggiornamento è fermo o se un accesso è scaduto; se il server non risponde l'app mostra l'ultima
lettura salvata e riprova da sola; "Apri in Outlook" cerca l'evento nell'Outlook del PC per ora e
titolo (serve Outlook aperto sul PC solo per quel pulsante, non per il calendario). Limiti noti:
non si vedono gli appuntamenti che stanno **solo** in un calendario locale di Outlook ("solo questo
computer") né i compleanni dei contatti, e le riunioni annullate spariscono invece di restare come
"Annullato:".

---

## 6. Il rilevatore degli accessi scaduti

Chiude il "falso verde" (vedi [ragionamenti](#1-i-ragionamenti-perché-è-fatto-così)).
`strumenti/rileva_accessi.py`, lanciato dal servizio ogni ~5 minuti, scrive in `stato/accessi.json`
lo stato di ogni casella, che l'agente usa per il pallino — **senza mai leggere un token**:

- **Caselle Microsoft (Outlook):** legge la bandierina "login da rifare" (`needsReauth`) nel
  registro di Outlook. Quando c'è → pallino **rosso** "Accesso scaduto sul telefono". Segnale netto.
- **Calendario Google (DAVx⁵):** DAVx⁵ non ha una bandierina simile e un login scaduto dà lo stesso
  errore di una rete assente. Ma il servizio lo spinge ogni 30 s e DAVx⁵ sincronizza da sé ogni
  15 min: se la sua ultima sincronizzazione **riuscita** si ferma molto oltre (mentre le caselle
  Outlook continuano, quindi non è la rete) la causa è quasi sempre il login scaduto →
  **giallo** oltre `DAVX5_GIALLO_S` (20 min), **rosso** oltre `DAVX5_ROSSO_S` (45 min). Se DAVx⁵
  mostra una notifica attiva che chiede di ricollegarsi, è rosso subito.

Le soglie si cambiano con le variabili d'ambiente `DAVX5_GIALLO_S` / `DAVX5_ROSSO_S` (nel servizio),
e la cadenza del controllo con `ACCESSI_OGNI` (giri di sincronizzazione fra un controllo e l'altro).

---

## 7. Manutenzione e diagnostica

- **Rifare un login scaduto**: `strumenti\schermo_telefono.py` sul PC (mette e toglie `PAUSA` da
  solo), rifai il login nell'app, chiudi la finestra.
- **Segni di vita** (sul server): `stato/battito.json` (il servizio è vivo? Outlook è davanti?) e
  `stato/accessi.json` (stato di ogni casella).
- **Log dell'app** (sul PC): `debug.log`; del tunnel: `agente_tunnel.log`.
- **Vedere cosa fa l'agente**: `curl` su `http://127.0.0.1:3180/api/calendari` dal server (con la
  chiave, vedi 4.5), oppure `agente_remoto.py prova` dal PC.
- **Aggiornare il codice del server**: con l'aggiornamento automatico (sezione 4.6) basta un push
  su `main`. A mano: `git pull`, ricopia `server/` in `/opt/oe-copilot` (mantieni `.env` e
  `agent/fonti.json`), `sudo docker compose up -d --build`, riavvia il servizio.
- **Backup**: i dati del telefono stanno in `/opt/oe-copilot/telefono/data` (contengono i login:
  trattali come un segreto). Uno snapshot coerente si fa con il telefono in `docker pause` per
  pochi secondi.

---

## 8. Sicurezza e privacy

- **Niente dati personali nel repository**: nomi, domini, indirizzi, calendari, sigle e schede
  stanno in `personalizzazione.json` (sul PC) e in `agent/fonti.json` + `.env` (sul server), tutti
  **esclusi da git**. `verifica.py` ha un controllo apposta che cerca i dati della personalizzazione
  nei file tracciati e fallisce se ne trova.
- **I login stanno nel telefono sul server**, non sul PC. I dati del telefono
  (`/opt/oe-copilot/telefono/data`) contengono i token degli accessi: proteggili come credenziali.
- **Niente esposto su internet**: l'agente e l'adb ascoltano solo sul localhost del server; il PC
  arriva con un tunnel SSH (chiave SSH) e una chiave dell'API cifrata per il tuo utente Windows.
  L'unica eccezione, facoltativa, è l'interfaccia nel browser (4.7): HTTPS e password davanti a
  una pagina che mostra i calendari, quindi la password va lunga e casuale. Il servizio `ui` in sé
  non è pubblicato e non ha password: ci arriva solo il Caddy che la chiede.
- **I token non si leggono**: il rilevatore degli accessi guarda solo bandierine ed età di
  sincronizzazione, mai il contenuto di un token.
- Non estrarre token/cookie dai browser e non usare client ID di altre app per aggirare un consenso.
