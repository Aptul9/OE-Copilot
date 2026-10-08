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
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

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
# Canali su cui DAVx5 chiede attenzione all'utente (login da rifare, errore di
# configurazione), da distinguere dai suoi avvisi di stato o di rete transitori.
DAVX5_CANALI_PROBLEMA = ("userInteraction", "syncProblems")


def _apri_ro(percorso):
    """Copia il db coi suoi file di appoggio e lo apre: Android lo tiene aperto."""
    tmp = tempfile.mkdtemp(dir="/dev/shm")
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
        for acc, _nr in c.execute("select accountId, needsReauth from tokenStore "
                                  "where needsReauth is not null and needsReauth != 0"):
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


def main():
    caselle = {}
    caselle.update(accessi_outlook())
    caselle.update(accessi_davx5())

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
