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

**Perché DAVx⁵ per i calendari Google.** Outlook per Android **non esporta** gli account Google nel
calendario di sistema di Android (l'interruttore "Sincronizza calendari" è disattivato per loro).
Per i calendari Google si usa quindi [DAVx⁵](https://www.davx5.com/) (CalDAV), con il login Google
fatto in un browser vero dentro il telefono (Google vieta il login nelle WebView interne).

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
 │   (promemoria Windows/Telegram)  │          │   └──────┬────────┘   │  Outlook+DAVx⁵ │ │
 └─────────────────────────────────┘          │          │ legge      └───────┬────────┘ │
                                               │   calendar.db (sola lettura) │          │
                                               │   servizio oe-copilot-sincronizza ◄──────┘
                                               │   (forza sync ogni 30s, rileva accessi)   │
                                               └──────────────────────────────────────────┘
```

- **telefono** = container redroid con l'app Outlook (caselle Microsoft) e DAVx⁵ (calendari
  Google). Le app copiano gli appuntamenti nel calendario di sistema di Android.
- **agent** = piccolo server HTTP che legge quel calendario e lo espone con un'API a chiave, in
  ascolto solo su `127.0.0.1:3180` del server.
- **servizio** = tiene aggiornato il telefono e scrive i "segni di vita" (`battito.json`,
  `accessi.json`).
- **app sul PC** = apre il tunnel SSH e chiede gli eventi all'agent; per il resto funziona come in
  modalità PC (griglia, colori, promemoria).

Il codice del server sta nella cartella **`server/`** del repository.

---

## 3. Installazione sul PC

### Requisiti
- Windows 10/11.
- **Outlook desktop classico** installato e configurato (serve per la modalità PC e per il pulsante
  "Apri in Outlook"; non il "Nuovo Outlook").
- **Python 3.12** (da python.org).
- Runtime **Microsoft Edge WebView2** (di norma già presente su Windows 11).
- Connessione a internet al primo avvio (FullCalendar arriva da una CDN).

### Passi
Dalla cartella dell'app, in un prompt dei comandi:

```bat
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python strumenti\applica_patch_pywebview.py
```

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

### 4.3 Avviare lo stack
```bash
sudo docker compose up -d        # avvia 'telefono' (redroid) e 'agent'
sudo docker ps                   # devono essere "Up" entrambi
```
Il primo avvio scarica le immagini. Se il telefono non parte, quasi sempre manca il modulo binder
(vedi 4.1).

### 4.4 Configurare il telefono virtuale
Lo schermo del telefono si vede dal **tuo PC** con [scrcpy](https://github.com/Genymobile/scrcpy),
attraverso un tunnel SSH verso la porta adb del telefono:
```bat
rem sul PC, in un prompt:
ssh -N -L 15555:127.0.0.1:5555 <utente@server>
rem in un altro prompt, nella cartella di scrcpy:
adb connect 127.0.0.1:15555
scrcpy -s 127.0.0.1:15555
```
(la porta adb `5555` è esposta solo sul localhost del server, come l'agente: si raggiunge solo col
tunnel).

Nel telefono:
1. **Impostazioni Android**: lingua e fuso orario corretti; schermo sempre acceso
   (`adb shell svc power stayon true`); togli l'ottimizzazione batteria (doze) a Outlook e DAVx⁵.
2. **Installa l'app Outlook** (prima parte Microsoft). L'APK va preso **arm64‑v8a** (redroid qui è
   64‑bit: gli APK armeabi‑v7a falliscono). Prima di installarlo conviene verificare che sia davvero
   l'app di Microsoft: la firma ha SHA‑1 (base64) `fcg80qvoM1YMKJZibjBwQcDfOno=` (è pubblica, la
   stessa del redirect `msauth://com.microsoft.office.outlook/…`). Poi la validità della firma la
   controlla Android all'installazione.
3. **Login delle caselle Microsoft** in Outlook. Per **ogni** casella, dopo averla aggiunta, apri
   *Impostazioni → Calendario → Sincronizza calendari* e **accendi l'interruttore**: è **per
   account**, e le caselle aggiunte dopo partono **spente** (è il motivo tipico per cui un
   calendario "non arriva").
4. **Calendari Google** (se ne hai): installa **DAVx⁵** e un browser vero (es. **Firefox**, perché
   Google blocca il login nella WebView interna). In DAVx⁵ aggiungi l'account Google via login nel
   browser, poi attiva la collezione del calendario come **sola lettura**.

> I login (password e MFA) li fai **tu**, dalla finestra di scrcpy. Durante un login crea il file
> `PAUSA` così il servizio non ti "ruba" lo schermo:
> `ssh <server> "sudo touch /opt/oe-copilot/telefono/PAUSA"` e rimuovilo alla fine.

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
   Prova: `ssh oecopilot hostname` deve entrare senza chiedere nulla.

2. **Chiave dell'API**, salvata cifrata per il tuo utente Windows (non passa mai dallo schermo):
   ```bat
   ssh oecopilot "grep ^API_KEY= /opt/oe-copilot/.env | cut -d= -f2" | .venv\Scripts\python agente_remoto.py imposta-chiave
   ```

3. In **`personalizzazione.json`** (sul PC) aggiungi la voce `agente_remoto` e togli dai
   `calendari_google` gli account che ora legge il server:
   ```json
   "agente_remoto": {"ssh": "oecopilot", "porta_remota": 3180, "porta_locale": 31800}
   ```
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

- **Rifare un login scaduto**: crea `telefono/PAUSA`, apri scrcpy, rifai il login nell'app, togli
  `PAUSA`.
- **Segni di vita** (sul server): `stato/battito.json` (il servizio è vivo? Outlook è davanti?) e
  `stato/accessi.json` (stato di ogni casella).
- **Log dell'app** (sul PC): `debug.log`; del tunnel: `agente_tunnel.log`.
- **Vedere cosa fa l'agente**: `curl` su `http://127.0.0.1:3180/api/calendari` dal server (con la
  chiave, vedi 4.5), oppure `agente_remoto.py prova` dal PC.
- **Aggiornare il codice del server**: `git pull`, ricopia `server/` in `/opt/oe-copilot`
  (mantieni `.env` e `agent/fonti.json`), `sudo docker compose up -d --build`, riavvia il servizio.
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
- **I token non si leggono**: il rilevatore degli accessi guarda solo bandierine ed età di
  sincronizzazione, mai il contenuto di un token.
- Non estrarre token/cookie dai browser e non usare client ID di altre app per aggirare un consenso.
