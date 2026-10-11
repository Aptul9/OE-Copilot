#!/usr/bin/env python3
"""Rileva, per ogni casella del telefono, se l'accesso e' scaduto.

Chiude il "falso verde": quando un login sul telefono scade (cambio password,
sessione revocata, MFA), il calendario Android resta com'era e l'app, che legge
quella copia, non se ne accorgerebbe. Qui si guardano solo fatti, mai un token:

  - Caselle Microsoft (Outlook): la bandierina "needsReauth" nel registro di
    Outlook. E' netta: quando c'e', il login e' da rifare -> rosso.

  - Calendario Google (DAVx5): DAVx5 non espone una bandierina simile, e un
    login Google scaduto da' lo stesso errore di una rete assente. Ma il servizio
    spinge DAVx5 ogni 30 s e DAVx5 sincronizza da se' ogni 15 min: se la sua
    ultima sincronizzazione RIUSCITA si ferma molto oltre (mentre le caselle
    Outlook continuano, quindi non e' la rete), la causa e' quasi sempre il login
    scaduto -> giallo se in ritardo, rosso se fermo da un pezzo. In piu', se
    DAVx5 ha una notifica attiva che chiede di ricollegarsi, e' rosso subito.

  - Freschezza di Outlook. Outlook non registra da nessuna parte quando ha
    scaricato dal server, e dalla versione coi servizi Google non ha più bisogno
    di stare in primo piano: le novità gli arrivano con le notifiche push (FCM)
    e le scarica anche a telefono bloccato (prova del 2026-10-11: quattro push
    ricevuti a schermo bloccato, il più recente seguito da 94 appuntamenti
    riscritti nel calendario in 9 secondi). Si misura quello che Outlook dice
    davvero: per ogni casella l'ultima riga "isSyncingForAccount" di
    calendarSync.log (a ogni giro il servizio forza la sincronizzazione e Outlook
    risponde per ciascuna), e se le notifiche push sono disponibili
    (notifications.log). Giallo se Outlook tace da più di OUTLOOK_GIALLO_S o ha
    sospeso la casella; rosso oltre OUTLOOK_ROSSO_S; giallo anche se le push non
    ci sono E Outlook non è in primo piano (allora scarica solo a schermo acceso).

  - Calendario Google (servizi Google del telefono): da dumpsys content, per
    "Account <email> com.google" la riga di com.android.calendar con l'esito
    dell'ultima sincronizzazione (SUCCESS/FAILURE) e la sua ora. Un fallimento
    isolato è la rete; se dura oltre GOOGLE_GIALLO_S senza un successo in mezzo è
    giallo, oltre GOOGLE_ROSSO_S rosso (login scaduto). L'ora dell'ultimo successo
    si ricorda in stato/google-sync.json fra un controllo e l'altro.

Il risultato -- per ogni casella uno stato (ok/avviso/errore) e un messaggio --
va in stato/accessi.json, che l'agente legge e usa per il pallino. I token non
vengono mai letti ne' scritti.

Uso: rileva_accessi.py [cartella_dati] [uscita.json]
  cartella_dati  radice dei dati del telefono (default: quella del telefono vero)
  uscita.json    dove scrivere (default: /opt/oe-copilot/stato/accessi.json)
"""
import base64
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from datetime import datetime

DATI = sys.argv[1] if len(sys.argv) > 1 else "/opt/oe-copilot/telefono/data"
USCITA = sys.argv[2] if len(sys.argv) > 2 else "/opt/oe-copilot/stato/accessi.json"
OUTLOOK = os.path.join(DATI, "data/com.microsoft.office.outlook/databases/acompliAcct.db")
DAVX5 = os.path.join(DATI, "data/at.bitfire.davdroid/databases/services.db")

# Container del telefono: serve solo per leggere le notifiche di DAVx5.
TELEFONO = os.environ.get("TELEFONO", "oe-copilot-telefono")
# DAVx5 sincronizza ogni 15 min: oltre queste eta' la mancata sincronizzazione
# non e' piu' un ritardo normale.
DAVX5_GIALLO_S = int(os.environ.get("DAVX5_GIALLO_S", "1200"))   # 20 min: in ritardo
DAVX5_ROSSO_S = int(os.environ.get("DAVX5_ROSSO_S", "2700"))     # 45 min: fermo davvero
# Freschezza di Outlook: il servizio forza la sincronizzazione ogni 30 s e Outlook
# risponde per ogni casella, quindi un silenzio di qualche minuto è già un guasto.
LOG_OUTLOOK = os.path.join(DATI, "data/com.microsoft.office.outlook/app_logs")
STATO_DIR = os.path.dirname(USCITA)
BATTITO = os.path.join(STATO_DIR, "battito.json")
GOOGLE_STATO = os.path.join(STATO_DIR, "google-sync.json")
OUTLOOK_GIALLO_S = int(os.environ.get("OUTLOOK_GIALLO_S", "600"))    # 10 min
OUTLOOK_ROSSO_S = int(os.environ.get("OUTLOOK_ROSSO_S", "1800"))     # 30 min
PUSH_RECENTE_S = 600     # una riga sulle push più vecchia di così non dice niente
GOOGLE_GIALLO_S = int(os.environ.get("GOOGLE_GIALLO_S", "900"))      # 15 min
GOOGLE_ROSSO_S = int(os.environ.get("GOOGLE_ROSSO_S", "2700"))       # 45 min
_GRAVITA = {"ok": 0, "avviso": 1, "errore": 2}
RIGA_CASELLA = re.compile(r"^\w\t(\S+)\t.*isSyncingForAccount: accountId=(\d+), "
                          r"hxAccountIsSyncing=\w+.*=> isSyncing=(true|false)")
RIGA_PUSH = re.compile(r"^\w\t(\S+)\t.*FCM push sync is (available|unavailable)")
# Canali su cui DAVx5 chiede attenzione all'utente (login da rifare, errore di
# configurazione), da distinguere dai suoi avvisi di stato o di rete transitori.
DAVX5_CANALI_PROBLEMA = ("userInteraction", "syncProblems")


def _apri_ro(percorso):
    """Copia il db coi suoi file di appoggio e lo apre: Android lo tiene aperto."""
    tmp = tempfile.mkdtemp(dir="/dev/shm" if os.path.isdir("/dev/shm") else None)
    os.chmod(tmp, 0o700)
    for suffisso in ("", "-journal", "-wal", "-shm"):
        if os.path.exists(percorso + suffisso):
            shutil.copy2(percorso + suffisso, os.path.join(tmp, "db" + suffisso))
    return tmp, sqlite3.connect(os.path.join(tmp, "db"))


def accessi_outlook():
    """{email: {stato, msg}} per le caselle Microsoft, dalla bandierina needsReauth.

    Si leggono solo email, identificativo dell'account e la colonna needsReauth,
    mai il valore del token.
    """
    if not os.path.exists(OUTLOOK):
        return {}
    tmp, c = _apri_ro(OUTLOOK)
    try:
        reauth = {e: False for (e,) in c.execute(
            "select distinct lower(email) from mailAccounts where email is not null")}
        # Il servizio MFA di Microsoft (adnotifications.windowsazure.com) chiede
        # spesso un nuovo accesso per registrare le informazioni di sicurezza: non
        # riguarda posta e calendario, che continuano a funzionare.
        for acc, _nr in c.execute("select accountId, needsReauth from tokenStore "
                                  "where needsReauth is not null and needsReauth != 0 "
                                  "and (resource is null "
                                  "or resource not like '%adnotifications.windowsazure.com%')"):
            try:
                chiaro = base64.b64decode(acc + "=" * (-len(acc) % 4),
                                          altchars=b"-_").decode("latin-1").lower()
            except (ValueError, UnicodeDecodeError):
                continue
            for email in reauth:
                if email in chiaro:
                    reauth[email] = True
        return {e: ({"stato": "errore",
                     "msg": "Accesso scaduto sul telefono: login da rifare in Outlook"}
                    if r else {"stato": "ok", "msg": ""})
                for e, r in reauth.items()}
    finally:
        c.close()
        shutil.rmtree(tmp, ignore_errors=True)


def davx5_chiede_login():
    """True se DAVx5 ha una notifica attiva che chiede di ricollegare l'account.

    Best-effort: se non si riesce a leggere le notifiche, si torna False e vale
    il solo criterio sull'eta' della sincronizzazione.
    """
    try:
        out = subprocess.run(
            ["docker", "exec", TELEFONO, "dumpsys", "notification", "--noredact"],
            capture_output=True, text=True, timeout=25).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    for blocco in out.split("NotificationRecord(")[1:]:
        if "at.bitfire.davdroid" not in blocco:
            continue
        if any(("channelId=" + ch) in blocco or ("mChannelId=" + ch) in blocco
               for ch in DAVX5_CANALI_PROBLEMA):
            return True
    return False


def accessi_davx5():
    """{email: {stato, msg}} per i calendari Google, da quanto DAVx5 non sincronizza."""
    if not os.path.exists(DAVX5):
        return {}
    tmp, c = _apri_ro(DAVX5)
    try:
        adesso = time.time()
        eta = {}
        for nome, ultima in c.execute(
                "select s.accountName, max(t.lastSync) from service s "
                "join collection col on col.serviceId = s.id "
                "join syncstats t on t.collectionId = col.id "
                "where s.type = 'caldav' and col.sync != 0 group by s.accountName"):
            if nome and ultima:
                eta[nome.lower()] = int(max(0, adesso - ultima / 1000.0))
    finally:
        c.close()
        shutil.rmtree(tmp, ignore_errors=True)
    if not eta:
        return {}
    problema = davx5_chiede_login()
    out = {}
    for email, e in eta.items():
        if problema:
            out[email] = {"stato": "errore",
                          "msg": "Accesso Google da rifare: DAVx⁵ chiede di ricollegare l'account sul telefono"}
        elif e > DAVX5_ROSSO_S:
            out[email] = {"stato": "errore",
                          "msg": "Calendario Google fermo da %d min sul telefono: probabile accesso scaduto, ricollega DAVx⁵" % (e // 60)}
        elif e > DAVX5_GIALLO_S:
            out[email] = {"stato": "avviso",
                          "msg": "Calendario Google non sincronizzato da %d min sul telefono" % (e // 60)}
        else:
            out[email] = {"stato": "ok", "msg": ""}
    return out


def _peggiore(a, b):
    """Dei due verdetti tiene il più grave, con tutti e due i messaggi."""
    if a is None:
        return b
    grave = b if _GRAVITA.get(b["stato"], 0) > _GRAVITA.get(a["stato"], 0) else a
    return {"stato": grave["stato"],
            "msg": " · ".join(dict.fromkeys(x for x in (a["msg"], b["msg"]) if x))}


def _epoca(testo):
    """Data/ora dei log di Outlook (2026-10-11T05:03:21.996+0000) -> secondi, None se illeggibile."""
    try:
        return datetime.strptime(testo, "%Y-%m-%dT%H:%M:%S.%f%z").timestamp()
    except ValueError:
        return None


def _coda(percorso, byte=600_000):
    """Le ultime righe intere di un log di Outlook, o None se non c'è.

    Quando il log è appena ruotato quello nuovo è quasi vuoto: si guarda anche la
    coda del precedente, altrimenti ogni casella sembrerebbe muta.
    """
    pezzi = []
    for nome in (percorso + ".1", percorso):
        try:
            with open(nome, "rb") as f:
                f.seek(0, os.SEEK_END)
                fine = f.tell()
                f.seek(max(0, fine - byte))
                testo = f.read().decode("utf-8", "replace")
        except OSError:
            continue
        pezzi.append(testo.split("\n", 1)[-1] if fine > byte else testo)
    if not pezzi or not os.path.exists(percorso):
        return None
    return "\n".join(pezzi[-2:])


def _verdetto_outlook(ultima, push, davanti, adesso, bloccato=False):
    """Verdetto per una casella: ultima = (secondi, sincronizza) o None, push = (secondi, disponibile) o None."""
    if ultima is None:
        return {"stato": "errore", "msg": "Outlook non conferma la sincronizzazione della casella "
                "da oltre %d min: Outlook fermo o sospeso sul telefono" % (OUTLOOK_ROSSO_S // 60)}
    quando, sincronizza = ultima
    eta = int(adesso - quando)
    if not sincronizza:
        return {"stato": "avviso", "msg": "Outlook ha sospeso la sincronizzazione della casella"}
    if eta > OUTLOOK_ROSSO_S:
        return {"stato": "errore", "msg": "Outlook non conferma la sincronizzazione della casella "
                "da %d min: Outlook fermo o sospeso sul telefono" % (eta // 60)}
    if eta > OUTLOOK_GIALLO_S:
        return {"stato": "avviso", "msg": "Outlook non conferma la sincronizzazione della casella "
                "da %d min" % (eta // 60)}
    if bloccato:
        return {"stato": "avviso", "msg": "Telefono bloccato: le cancellazioni e le modifiche fatte in Outlook "
                "arrivano in ritardo, sbloccalo dalla pagina Telefono"}
    if push and not push[1] and adesso - push[0] < PUSH_RECENTE_S and not davanti:
        return {"stato": "avviso", "msg": "Outlook non riceve le notifiche push dal server e non "
                "è in primo piano: a telefono bloccato le novità non arrivano"}
    return {"stato": "ok", "msg": ""}


def freschezza_outlook(adesso=None):
    """{email: {stato, msg}} per le caselle Microsoft: Outlook risponde ancora?

    Non misura il momento dell'ultimo scarico dal server, che Outlook non scrive:
    misura che Outlook sia vivo e con la casella attiva, e che le notifiche push
    (il suo modo di sapere delle novità) funzionino. Se i log non ci sono non si
    dice niente: un controllo che non può guardare non deve dare un verdetto.
    """
    adesso = adesso or time.time()
    sync = _coda(os.path.join(LOG_OUTLOOK, "calendarSync.log"))
    if sync is None or not os.path.exists(OUTLOOK):
        return {}
    ultima = {}
    for riga in sync.splitlines():
        m = RIGA_CASELLA.match(riga)
        if m and _epoca(m.group(1)):
            ultima[m.group(2)] = (_epoca(m.group(1)), m.group(3) == "true")
    push = None
    for riga in (_coda(os.path.join(LOG_OUTLOOK, "notifications.log")) or "").splitlines():
        m = RIGA_PUSH.match(riga)
        if m and _epoca(m.group(1)):
            push = (_epoca(m.group(1)), m.group(2) == "available")
    try:
        with open(BATTITO, encoding="utf-8") as f:
            battito = json.load(f)
        davanti = bool(battito.get("outlook_davanti", True))
        bloccato = bool(battito.get("bloccato", False))
    except (OSError, ValueError, AttributeError):
        davanti, bloccato = True, False
    tmp, c = _apri_ro(OUTLOOK)
    try:
        caselle = {int(i): e for i, e in c.execute(
            "select accountID, lower(email) from mailAccounts "
            "where hxAccountId is not null and email is not null")}
    finally:
        c.close()
        shutil.rmtree(tmp, ignore_errors=True)
    return {e: _verdetto_outlook(ultima.get(str(i)), push, davanti, adesso, bloccato)
            for i, e in caselle.items()}


RIGA_GOOGLE = re.compile(r"^Account (\S+) u\d+ com\.google\s*$")
RIGA_ESITO = re.compile(r"^com\.android\.calendar\s.*?\b(?:[A-Z]+) (SUCCESS|FAILURE)\b")
RIGA_ORA = re.compile(r"(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d)\s*$")


def esiti_google(dumpsys):
    """{email: (ora di telefono, riuscita)} dall'ultima sincronizzazione del calendario Google.

    Si leggono solo l'indirizzo e la riga di com.android.calendar: gli altri
    adattatori di Google (contatti con "OTHER FAILURE internal-error" sempre,
    fitness, giochi...) non riguardano il calendario.
    """
    esiti, conto, riga_esito = {}, None, None
    for riga in dumpsys.splitlines():
        m = RIGA_GOOGLE.match(riga)
        if m:
            conto, riga_esito = m.group(1).lower(), None
            continue
        if riga.startswith("Account "):
            conto = None
        if conto is None:
            continue
        m = RIGA_ESITO.match(riga)
        if m:
            riga_esito = m.group(1) == "SUCCESS"
            continue
        if riga.startswith("com."):   # un altro adattatore: l'ora che segue non è del calendario
            riga_esito = None
        if riga_esito is not None:
            m = RIGA_ORA.search(riga)
            if m:
                esiti[conto] = (datetime.strptime(m.group(1), "%Y-%m-%d %H:%M:%S"), riga_esito)
                riga_esito = None
    return esiti


def accessi_google(dumpsys, ora_telefono, adesso, ricordo):
    """{email: {stato, msg}} per i calendari Google, e il ricordo aggiornato.

    ricordo = {email: secondi dell'ultimo successo visto}. L'ora dell'esito è del
    telefono: la si porta ai secondi del server con la differenza fra le due ore.
    """
    ricordo = dict(ricordo)
    out = {}
    for email, (quando, riuscita) in esiti_google(dumpsys).items():
        epoca = adesso - (ora_telefono - quando).total_seconds()
        if riuscita:
            ricordo[email] = max(ricordo.get(email, 0), epoca)
        ultimo_ok = ricordo.setdefault(email, adesso)   # mai visto: si parte da ora
        senza = int(adesso - ultimo_ok)
        if not riuscita and senza > GOOGLE_ROSSO_S:
            out[email] = {"stato": "errore", "msg": "Google non sincronizza da %d min sul telefono: "
                          "probabile accesso scaduto, rifai il login Google" % (senza // 60)}
        elif not riuscita and senza > GOOGLE_GIALLO_S:
            out[email] = {"stato": "avviso", "msg": "Google non sincronizza da %d min sul telefono" % (senza // 60)}
        elif adesso - epoca > GOOGLE_GIALLO_S:
            out[email] = {"stato": "avviso", "msg": "La sincronizzazione Google sul telefono non parte "
                          "da %d min" % ((adesso - epoca) // 60)}
        else:
            out[email] = {"stato": "ok", "msg": ""}
    return out, ricordo


def verifica_google(adesso=None):
    """Legge dumpsys content dal telefono e applica accessi_google(). {} se non si riesce."""
    adesso = adesso or time.time()
    try:
        dump = subprocess.run(["docker", "exec", TELEFONO, "dumpsys", "content"],
                              capture_output=True, text=True, timeout=25).stdout
        ora = subprocess.run(["docker", "exec", TELEFONO, "date", "+%Y-%m-%d %H:%M:%S"],
                             capture_output=True, text=True, timeout=10).stdout.strip()
        ora_telefono = datetime.strptime(ora, "%Y-%m-%d %H:%M:%S")
    except (OSError, ValueError, subprocess.SubprocessError):
        return {}
    try:
        with open(GOOGLE_STATO, encoding="utf-8") as f:
            ricordo = {k: float(v) for k, v in json.load(f).items()}
    except (OSError, ValueError, AttributeError):
        ricordo = {}
    out, ricordo = accessi_google(dump, ora_telefono, adesso, ricordo)
    if out:
        tmp = GOOGLE_STATO + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(ricordo, f)
        os.replace(tmp, GOOGLE_STATO)
    return out


def main():
    caselle = {}
    for nome, controllo in (("accessi Outlook", accessi_outlook), ("freschezza Outlook", freschezza_outlook),
                            ("DAVx5", accessi_davx5), ("Google", verifica_google)):
        try:
            fonte = controllo()
        except Exception as e:   # un controllo che si rompe non deve fermare gli altri
            print("rilevatore: %s non riuscito (%s)" % (nome, e), file=sys.stderr)
            continue
        for email, verdetto in fonte.items():
            caselle[email] = _peggiore(caselle.get(email), verdetto)

    dati = {"generato": int(time.time()), "caselle": caselle}
    os.makedirs(os.path.dirname(USCITA), exist_ok=True)
    tmp = USCITA + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(dati, f)
    os.replace(tmp, USCITA)

    problemi = sorted("%s (%s)" % (e, v["stato"]) for e, v in caselle.items()
                      if v["stato"] != "ok")
    print("accessi: %d caselle esaminate, da sistemare: %s"
          % (len(caselle), ", ".join(problemi) or "nessuna"))


if __name__ == "__main__":
    main()
