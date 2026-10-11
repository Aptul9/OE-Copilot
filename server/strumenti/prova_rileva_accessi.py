#!/usr/bin/env python3
"""Prova del rilevatore: dati finti, nessun telefono. La lancia la Action a ogni modifica.

Verifica che con Outlook vivo e le push attive non ci sia nessun avviso (anche a
schermo bloccato), e che una fonte ferma, sospesa o senza push accenda l'avviso
giusto con il testo giusto.
"""
import os
import sqlite3
import sys
import tempfile
import time
from datetime import datetime, timedelta, timezone

radice = tempfile.mkdtemp()
dati = os.path.join(radice, "telefono")
stato = os.path.join(radice, "stato")
log = os.path.join(dati, "data/com.microsoft.office.outlook/app_logs")
os.makedirs(log)
os.makedirs(stato)
os.makedirs(os.path.join(dati, "data/com.microsoft.office.outlook/databases"))
sys.argv = ["rileva_accessi.py", dati, os.path.join(stato, "accessi.json")]
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rileva_accessi as r  # noqa: E402

db = sqlite3.connect(r.OUTLOOK)
db.execute("create table mailAccounts (accountID integer, hxAccountId text, email text)")
db.executemany("insert into mailAccounts values (?, ?, ?)", [
    (32768, "roar_Hx", "Roar@example.com"), (32769, None, "roar@example.com"),
    (32772, "band_Hx", "band@example.com"), (32773, None, "band@example.com")])
db.commit()
db.close()

ADESSO = time.time()


def stamp(secondi_fa):
    t = datetime.fromtimestamp(ADESSO - secondi_fa, timezone.utc)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + "%03d+0000" % (t.microsecond // 1000)


def riga_casella(secondi_fa, idh, sincronizza="true"):
    return ("D\t%s\t[ci=x]\tSyncAdapterThread\toutlook.calendarSync > CalendarSyncAccou\t"
            "isSyncingForAccount: accountId=%d, hxAccountIsSyncing=%s, calendarSyncAccount!=null=true, "
            "isSyncable>0=true => isSyncing=%s\n" % (stamp(secondi_fa), idh, sincronizza, sincronizza))


def riga_push(secondi_fa, disponibile=True):
    return ("I\t%s\t[ci=x]\tOM_APP_SESSION_MA\toutlook.Notifications > FcmTokenReaderWr\t"
            "FCM push sync is %s\n" % (stamp(secondi_fa),
                                       "available" if disponibile else "unavailable due to missing token"))


def scrivi(nome, righe):
    with open(os.path.join(log, nome), "w", encoding="utf-8") as f:
        f.writelines(righe)


def battito(davanti):
    with open(r.BATTITO, "w", encoding="utf-8") as f:
        f.write('{"quando": %d, "pausa": false, "telefono": true, "outlook_davanti": %s}'
                % (ADESSO, "true" if davanti else "false"))


def verdetti():
    return r.freschezza_outlook(ADESSO)


def controlla(nome, v, email, stato_atteso, testo=""):
    trovato = v[email]
    assert trovato["stato"] == stato_atteso and testo in trovato["msg"], \
        "%s: %s atteso %s/%r, trovato %r" % (nome, email, stato_atteso, testo, trovato)
    print("ok  ", nome, "->", email, trovato["stato"], trovato["msg"][:70])


# 1. Outlook vivo, push attive, telefono bloccato (non davanti): nessun avviso
battito(False)
scrivi("calendarSync.log", [riga_casella(60, 32768), riga_casella(55, 32772),
                            riga_casella(30, 32768), riga_casella(25, 32772)])
scrivi("notifications.log", [riga_push(20)])
v = verdetti()
controlla("bloccato e vivo", v, "roar@example.com", "ok")
controlla("bloccato e vivo", v, "band@example.com", "ok")

# 2. Una casella muta da 15 minuti: giallo, con i minuti
scrivi("calendarSync.log", [riga_casella(900, 32772), riga_casella(30, 32768)])
controlla("casella muta 15 min", verdetti(), "band@example.com", "avviso", "da 15 min")
controlla("l'altra resta ok", verdetti(), "roar@example.com", "ok")

# 3. Muta da un'ora: rosso
scrivi("calendarSync.log", [riga_casella(3600, 32772), riga_casella(30, 32768)])
controlla("casella muta 1 h", verdetti(), "band@example.com", "errore", "fermo o sospeso")

# 4. Outlook dice di aver sospeso la casella: giallo
scrivi("calendarSync.log", [riga_casella(30, 32772, "false"), riga_casella(30, 32768)])
controlla("casella sospesa", verdetti(), "band@example.com", "avviso", "sospeso")

# 5. Nessuna riga per la casella (log ruotato da poco, la coda non la contiene): rosso
scrivi("calendarSync.log", [riga_casella(30, 32768)])
controlla("casella assente dal log", verdetti(), "band@example.com", "errore")

# 6. Push assenti e Outlook non davanti: giallo; con Outlook davanti: ok
scrivi("calendarSync.log", [riga_casella(30, 32768), riga_casella(30, 32772)])
scrivi("notifications.log", [riga_push(20, disponibile=False)])
controlla("senza push e non davanti", verdetti(), "roar@example.com", "avviso", "notifiche push")
battito(True)
controlla("senza push ma davanti", verdetti(), "roar@example.com", "ok")
battito(False)

# 7. Una riga sulle push vecchia di un'ora non dice niente
scrivi("notifications.log", [riga_push(3600, disponibile=False)])
controlla("riga push vecchia", verdetti(), "roar@example.com", "ok")

# 8. Log ruotato: il corrente è vuoto, le righe stanno in calendarSync.log.1
scrivi("calendarSync.log", [])
with open(os.path.join(log, "calendarSync.log.1"), "w", encoding="utf-8") as f:
    f.writelines([riga_casella(40, 32768), riga_casella(40, 32772)])
controlla("log appena ruotato", verdetti(), "band@example.com", "ok")

# 9. Log di Outlook assenti: nessun verdetto invece di un falso rosso o verde
os.remove(os.path.join(log, "calendarSync.log"))
assert verdetti() == {}, verdetti()
print("ok   log assenti -> nessun verdetto")

# 10. Google: dumpsys finto, ore del telefono
DUMP = """Sync Status
Account Antonio.Maritato@roarinc.com u0 com.microsoft.office.outlook.USER_ACCOUNT
=======================================================================
com.android.calendar  1         true     Total  0    0     4466  4     0     8     4478  0     0    1m14s  PERIODIC SUCCESS
                                         Today             685                     685              10s    2026-10-11 07:09:39
Account g@example.com u0 com.google
=======================================================================
com.android.calendar                     1         true     Total  1    0     242  3     0     478   724  1     1    20m39s  %(esito)s
                                                            Today             173              346   519             14m51s  %(ora)s
                                                            Yestr  1          69   3           132   205  1     1    5m47s
com.android.contacts                     -1        false    Total  0    0     0    0     0     1     1    1     0    0s      OTHER FAILURE
                                                            Today                                                    0s      2026-10-10 21:22:56
                                                            Yestr                              1     1    1          0s      internal-error
"""
TELEFONO = datetime(2026, 10, 11, 9, 0, 0)


def google(esito, minuti_fa, ricordo):
    ora = (TELEFONO - timedelta(minutes=minuti_fa)).strftime("%Y-%m-%d %H:%M:%S")
    return r.accessi_google(DUMP % {"esito": esito, "ora": ora}, TELEFONO, ADESSO, ricordo)


out, ric = google("OTHER SUCCESS", 1, {})
assert out["g@example.com"]["stato"] == "ok", out
print("ok   google riuscito 1 min fa -> ok (il fallimento dei contatti non conta)")
out, _ = google("OTHER FAILURE", 1, {"g@example.com": ADESSO - 120})
assert out["g@example.com"]["stato"] == "ok", out
print("ok   google fallimento isolato -> ok")
out, _ = google("OTHER FAILURE", 1, {"g@example.com": ADESSO - 1800})
assert out["g@example.com"]["stato"] == "avviso" and "30 min" in out["g@example.com"]["msg"], out
print("ok   google in errore da 30 min ->", out["g@example.com"]["msg"])
out, _ = google("OTHER FAILURE", 1, {"g@example.com": ADESSO - 7200})
assert out["g@example.com"]["stato"] == "errore" and "accesso scaduto" in out["g@example.com"]["msg"], out
print("ok   google in errore da 2 h ->", out["g@example.com"]["msg"])
out, _ = google("OTHER SUCCESS", 40, {})
assert out["g@example.com"]["stato"] == "avviso" and "non parte" in out["g@example.com"]["msg"], out
print("ok   google fermo da 40 min ->", out["g@example.com"]["msg"])
out, ric = google("OTHER FAILURE", 1, {})
assert out["g@example.com"]["stato"] == "ok" and ric["g@example.com"] == ADESSO, (out, ric)
print("ok   google fallimento mai visto un successo -> parte da ora")

# 11. La somma dei verdetti tiene il più grave e unisce i messaggi
a = {"stato": "avviso", "msg": "uno"}
b = {"stato": "errore", "msg": "due"}
assert r._peggiore(a, b) == {"stato": "errore", "msg": "uno · due"}, r._peggiore(a, b)
print("ok   verdetti uniti")
print("TUTTO OK")
