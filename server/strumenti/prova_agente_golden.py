#!/usr/bin/env python3
"""Prova dell'agente su un calendario di Android inventato: ricorrenze, annullamenti, fusi, doppioni. La lancia la Action.

Costruisce un calendar.db come quello del telefono (tabelle Calendars, Events, Attendees) con dati inventati, avvia
l'agente vero (agent.py, HTTP) su una porta libera, chiede /api/calendari e /api/eventi per alcune finestre e confronta le
risposte con quelle registrate in golden/agente.json (la risposta intera dell'agente: è anche quello che il servizio ui
e il gateway ricevono nelle loro prove). In più controlla a parte, senza il file registrato, i fatti che devono valere
(l'ora locale prima e dopo il cambio dell'ora, un'occorrenza annullata, una spostata, i doppioni).

Si esegue nell'immagine dell'agente (ha le librerie e il fuso di Roma), con il codice dell'agente in /app:
    docker run --rm -v "$PWD/server/agent:/app:ro" -v "$PWD/server/strumenti:/prove:ro" oe-copilot-agent python /prove/prova_agente_golden.py

Con --aggiorna riscrive il file registrato (montare /prove in scrittura): lo si fa solo dopo aver letto la differenza e
deciso che il risultato nuovo è quello giusto. Con --db <cartella> --fonti <fonti.json> --finestre <finestre.json> --uscita
<file> legge un calendar.db vero (una copia) al posto di quello inventato e scrive le risposte senza confrontarle: serve a
registrare e rileggere la copia congelata del telefono (prove/golden/ del kit, fuori dal repository).
"""
import json
import os
import re
import sqlite3
import sys
import tempfile
import threading
import urllib.request
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

os.environ.setdefault("TZ", "Europe/Rome")
QUI = os.path.dirname(os.path.abspath(__file__))
REGISTRATO = os.path.join(QUI, "golden", "agente.json")
CODICE = "/app" if os.path.exists("/app/agent.py") else os.path.abspath(os.path.join(QUI, "..", "agent"))
CHIAVE = "chiave-di-prova"


# ---- il calendar.db inventato ------------------------------------------------------------------------------------------
def ms(anno, mese, giorno, ora=0, minuto=0, fuso="Europe/Rome"):
    """Istante di Android: millisecondi UTC di un'ora locale."""
    locale = datetime(anno, mese, giorno, ora, minuto, tzinfo=ZoneInfo(fuso))
    return int(locale.astimezone(timezone.utc).timestamp() * 1000)


def giorno_utc(anno, mese, giorno):
    """Android salva i giorni interi a mezzanotte UTC."""
    return int(datetime(anno, mese, giorno, tzinfo=timezone.utc).timestamp() * 1000)


def riga(_id, calendario, titolo, inizio, fine=None, fuso="Europe/Rome", durata=None, tutto=0, **altri):
    r = dict(_id=_id, _sync_id=None, calendar_id=calendario, title=titolo, eventLocation=None, description=None,
             eventStatus=1, dtstart=inizio, dtend=fine, eventTimezone=fuso, eventEndTimezone=None, duration=durata,
             allDay=tutto, rrule=None, rdate=None, exrule=None, exdate=None, original_id=None, original_sync_id=None,
             originalInstanceTime=None, originalAllDay=None, organizer=None, deleted=0)
    r.update(altri)
    return r


OUTLOOK, GOOGLE = "com.microsoft.office.outlook", "com.google"
# _id, account, tipo, nome, principale, cancellato, sincronizza
CALENDARI = [
    (1, "mario@alfa.example", OUTLOOK, "Calendar", 1, 0, 1),
    (2, "mario@alfa.example", OUTLOOK, "Birthdays", 0, 0, 1),        # non principale: l'agente non lo legge
    (3, "mario@beta.example", OUTLOOK, "Calendar", 1, 0, 1),
    (4, "mario@beta.example", OUTLOOK, "Shared Team", 0, 0, 1),      # elencato nelle fonti: si legge
    (5, "mario@gmail.example", GOOGLE, "mario@gmail.example", 1, 0, 1),
    (6, "vecchio@alfa.example", OUTLOOK, "Calendar", 1, 1, 1),       # calendario cancellato
    (7, "altro@example.org", "com.example.sync", "Calendar", 1, 0, 1),   # account di un'altra app
]

FONTI = [
    {"tipo": "android", "id": "android::mario@alfa.example", "nome": "Alfa", "account": "mario@alfa.example"},
    {"tipo": "android", "id": "android::mario@beta.example", "nome": "Beta", "account": "mario@beta.example",
     "calendari": ["Calendar", "Shared Team"]},
    {"tipo": "android", "id": "android::mario@gmail.example", "nome": "Gmail", "account": "mario@gmail.example"},
    {"tipo": "android", "id": "android::assente@alfa.example", "nome": "Assente", "account": "assente@alfa.example"},
]

TEAMS = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0"
EVENTI = [
    # Alfa. Serie lun-mer-ven alle 9:00 per 15 minuti, a cavallo del cambio dell'ora (25/10/2026); Outlook scrive anche
    # due occorrenze a sé (19/10 e 2/11), con luogo e link: ne resta una per giorno, quella di Outlook
    riga(10, 1, "Standup", ms(2026, 10, 5, 9), None, durata="P900S", _sync_id="S-standup", organizer="anna@alfa.example",
         rrule="FREQ=WEEKLY;BYDAY=MO,WE,FR;UNTIL=20261113T080000Z"),
    riga(11, 1, "Standup", ms(2026, 10, 19, 9), ms(2026, 10, 19, 9, 15), eventLocation="Sala Rossa",
         description=TEAMS, organizer="anna@alfa.example"),
    riga(12, 1, "Standup", ms(2026, 11, 2, 9), ms(2026, 11, 2, 9, 15), eventLocation="Sala Rossa",
         description=TEAMS, organizer="anna@alfa.example"),
    # un singolo con il link nel luogo (con &amp;) e un secondo link protetto nella descrizione, organizzatore e invitati
    riga(20, 1, "Planning trimestrale", ms(2026, 10, 21, 14), ms(2026, 10, 21, 15, 30), _sync_id="p1",
         eventLocation=TEAMS + "?x=1&amp;y=2", organizer="Anna.Rossi@Alfa.Example",
         description="<html>Entra: " + TEAMS + "%3Chttps://eur01.safelinks.protection.outlook.com/?url=x%3E</html>"),
    riga(21, 1, "FW: Planning trimestrale", ms(2026, 10, 21, 14), ms(2026, 10, 21, 15, 30)),   # inoltro dello stesso
    # giorni interi: uno, e cinque di fila
    riga(30, 1, "Workshop", giorno_utc(2026, 10, 22), giorno_utc(2026, 10, 23), fuso="UTC", tutto=1),
    riga(31, 1, "Ferie", giorno_utc(2026, 10, 26), giorno_utc(2026, 10, 31), fuso="UTC", tutto=1),
    # a cavallo della mezzanotte, e nel giorno del cambio dell'ora (alle 3:00 legali si torna alle 2:00)
    riga(40, 1, "Deploy notturno", ms(2026, 10, 24, 22), ms(2026, 10, 25, 1, 30)),
    riga(41, 1, "Pranzo", ms(2026, 10, 25, 12), ms(2026, 10, 25, 13)),
    # fusi diversi: una serie a New York (il cambio dell'ora americano è il 1/11, quello italiano il 25/10) e un orario UTC
    riga(50, 1, "Sync New York", ms(2026, 10, 20, 10, fuso="America/New_York"), None, fuso="America/New_York",
         durata="P3600S", _sync_id="S-ny", rrule="FREQ=WEEKLY;BYDAY=TU;COUNT=4"),
    riga(51, 1, "Call UTC", ms(2026, 10, 28, 8, fuso="UTC"), ms(2026, 10, 28, 9, fuso="UTC"), fuso="UTC"),
    # eccezioni di una serie scritte da Outlook: EXDATE con fuso, EXDATE in UTC, occorrenza annullata, occorrenza spostata
    riga(60, 1, "Pranzo di squadra", ms(2026, 10, 8, 12, 30), None, durata="P3600S", _sync_id="S-pranzo",
         rrule="FREQ=WEEKLY;BYDAY=TH;COUNT=6", exdate="TZID=Europe/Rome;20261029T123000"),
    riga(61, 1, "Allineamento", ms(2026, 10, 9, 16), None, durata="P1800S", _sync_id="S-allin",
         rrule="FREQ=WEEKLY;BYDAY=FR;COUNT=6", exdate="20261023T140000Z,20261106T150000Z"),
    riga(70, 1, "Retro", ms(2026, 10, 9, 16, 30), None, durata="P3600S", _sync_id="S-retro",
         rrule="FREQ=WEEKLY;BYDAY=FR;COUNT=5"),
    riga(71, 1, "Retro", ms(2026, 10, 23, 16, 30), ms(2026, 10, 23, 17, 30), eventStatus=2, original_id=70,
         originalInstanceTime=ms(2026, 10, 23, 16, 30)),
    riga(80, 1, "Review", ms(2026, 10, 13, 11), None, durata="P3600S", _sync_id="S-review",
         rrule="FREQ=WEEKLY;BYDAY=TU;COUNT=4"),
    riga(81, 1, "Review (spostata)", ms(2026, 10, 27, 15), ms(2026, 10, 27, 16), original_id=80,
         originalInstanceTime=ms(2026, 10, 27, 11)),
    # senza titolo, annullato, cancellato, solo con la durata, due copie dello stesso (ne resta una: quella col luogo)
    riga(90, 1, "", ms(2026, 10, 20, 17), ms(2026, 10, 20, 17, 30)),
    riga(91, 1, "Annullata", ms(2026, 10, 20, 18), ms(2026, 10, 20, 19), eventStatus=2),
    riga(92, 1, "Cancellata", ms(2026, 10, 20, 19), ms(2026, 10, 20, 20), deleted=1),
    riga(93, 1, "Sync breve", ms(2026, 10, 20, 10), None, durata="P900S"),
    riga(94, 1, "Doppia", ms(2026, 10, 22, 9), ms(2026, 10, 22, 10)),
    riga(95, 1, "Doppia", ms(2026, 10, 22, 9), ms(2026, 10, 22, 10), eventLocation="Sala Blu"),
    # cose che l'agente non deve leggere: calendario non principale, calendario cancellato, account di un'altra app
    riga(96, 2, "Compleanno ignorato", ms(2026, 10, 20, 12), ms(2026, 10, 20, 13)),
    riga(97, 6, "Calendario cancellato", ms(2026, 10, 20, 12), ms(2026, 10, 20, 13)),
    riga(98, 7, "Altra app", ms(2026, 10, 20, 12), ms(2026, 10, 20, 13)),
    # Beta: la stessa riunione dell'altra casella, due inviti, un calendario condiviso
    riga(100, 3, "Standup", ms(2026, 10, 19, 9), ms(2026, 10, 19, 9, 15), organizer="anna@alfa.example"),
    riga(101, 3, "Pranzo con cliente", ms(2026, 10, 21, 13), ms(2026, 10, 21, 14), eventLocation="Ristorante",
         organizer="mario@beta.example"),
    riga(102, 4, "Offsite di team", ms(2026, 10, 23, 9), ms(2026, 10, 23, 17), organizer="capo@beta.example"),
    riga(103, 3, "Planning trimestrale", ms(2026, 10, 21, 14), ms(2026, 10, 21, 15, 30),
         organizer="Anna.Rossi@Alfa.Example"),
    # Gmail (servizi Google del telefono): un singolo, la stessa riunione di Alfa, un compleanno annuale di un giorno intero
    riga(200, 5, "Dentista", ms(2026, 10, 22, 10), ms(2026, 10, 22, 11), organizer="mario@gmail.example"),
    riga(201, 5, "Planning trimestrale", ms(2026, 10, 21, 14), ms(2026, 10, 21, 15, 30), organizer="anna.rossi@alfa.example"),
    riga(202, 5, "Compleanno Luca", giorno_utc(2026, 10, 24), None, fuso="UTC", tutto=1, durata="P1D",
         rrule="FREQ=YEARLY;WKST=SU", _sync_id="S-compleanno"),
]

# evento, nome, email, relazione (2 = organizzatore)
INVITATI = [
    (20, "Anna Rossi", "Anna.Rossi@Alfa.Example", 2),
    (20, "Mario", "Mario@Alfa.Example", 1),
    (20, "Mario", "mario@alfa.example", 1),
    (20, "Luca", "luca@beta.example", 1),
    (20, "Senza posta", None, 1),
    (100, "Anna Rossi", "anna@alfa.example", 2),
    (103, "Mario", "mario@beta.example", 1),
]

# inizio e fine come le manda il telefono all'agente: con o senza scarto
FINESTRE = [
    ["2026-10-19T00:00:00", "2026-11-09T00:00:00"],             # tre settimane a cavallo del cambio dell'ora
    ["2026-10-25T00:00:00", "2026-10-26T00:00:00"],             # il giorno del cambio dell'ora
    ["2026-11-02T00:00:00+01:00", "2026-11-03T00:00:00+01:00"],     # con lo scarto
    ["2026-10-01T00:00:00", "2026-12-31T00:00:00"],             # il trimestre
]


def costruisci_db(cartella):
    percorso = os.path.join(cartella, "calendar.db")
    c = sqlite3.connect(percorso)
    c.executescript("""
        CREATE TABLE Calendars (_id INTEGER PRIMARY KEY, account_name TEXT, account_type TEXT, calendar_displayName TEXT,
                                isPrimary INTEGER, deleted INTEGER DEFAULT 0, sync_events INTEGER DEFAULT 1);
        CREATE TABLE Events (_id INTEGER PRIMARY KEY, _sync_id TEXT, calendar_id INTEGER, title TEXT, eventLocation TEXT,
                             description TEXT, eventStatus INTEGER, dtstart INTEGER, dtend INTEGER, eventTimezone TEXT,
                             eventEndTimezone TEXT, duration TEXT, allDay INTEGER DEFAULT 0, rrule TEXT, rdate TEXT,
                             exrule TEXT, exdate TEXT, original_id INTEGER, original_sync_id TEXT,
                             originalInstanceTime INTEGER, originalAllDay INTEGER, organizer TEXT, deleted INTEGER DEFAULT 0);
        CREATE TABLE Attendees (_id INTEGER PRIMARY KEY, event_id INTEGER, attendeeName TEXT, attendeeEmail TEXT,
                                attendeeRelationship INTEGER);
    """)
    c.executemany("INSERT INTO Calendars VALUES (?,?,?,?,?,?,?)", CALENDARI)
    for r in EVENTI:
        c.execute("INSERT INTO Events (%s) VALUES (%s)" % (",".join(r), ",".join("?" * len(r))), list(r.values()))
    c.executemany("INSERT INTO Attendees (event_id, attendeeName, attendeeEmail, attendeeRelationship) VALUES (?,?,?,?)",
                  INVITATI)
    c.commit()
    c.close()


# ---- l'agente vero, su una porta libera -----------------------------------------------------------------------------------
def avvia_agente(cartella_db, fonti):
    base = tempfile.mkdtemp()
    with open(os.path.join(base, "fonti.json"), "w", encoding="utf-8") as f:
        json.dump(fonti, f)
    os.environ.update(FONTI=os.path.join(base, "fonti.json"), ANDROID_DB=cartella_db, API_KEY=CHIAVE,
                      BATTITO=os.path.join(base, "nessun-battito.json"), ACCESSI=os.path.join(base, "nessun-accesso.json"))
    sys.path.insert(0, CODICE)
    import agent
    httpd = agent.ThreadingHTTPServer(("127.0.0.1", 0), agent.Gestore)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return "http://127.0.0.1:%d" % httpd.server_address[1]


def chiedi(base, percorso):
    r = urllib.request.Request(base + percorso, headers={"Authorization": "Bearer " + CHIAVE})
    with urllib.request.urlopen(r, timeout=30) as risposta:
        return json.loads(risposta.read().decode("utf-8"))


def normalizza(dati):
    """L'ora del giorno cambia a ogni lettura: si toglie, il resto deve restare identico."""
    testo = json.dumps(dati, ensure_ascii=False, sort_keys=True)
    testo = re.sub(r"alle \d\d:\d\d", "alle HH:MM", testo)
    dati = json.loads(testo)
    if isinstance(dati, dict) and "generato" in dati:
        dati["generato"] = "ORA"
    return dati


def leggi_tutto(base, finestre):
    from urllib.parse import quote
    out = {"calendari": normalizza(chiedi(base, "/api/calendari")), "eventi": {}}
    for inizio, fine in finestre:
        chiave = "%s|%s" % (inizio, fine)
        out["eventi"][chiave] = normalizza(chiedi(base, "/api/eventi?inizio=%s&fine=%s" % (quote(inizio), quote(fine))))
    return out


# ---- i fatti che devono valere, a parte il file registrato -------------------------------------------------------------------
esiti = 0


def verifica(nome, ok, dettaglio=""):
    global esiti
    print(("ok   " if ok else "KO   ") + nome + ("" if ok else "  -> " + str(dettaglio)))
    esiti += 0 if ok else 1


def fatti(letto):
    ev = letto["eventi"]["2026-10-19T00:00:00|2026-11-09T00:00:00"]["eventi"]
    per_titolo = {}
    for e in ev:
        per_titolo.setdefault(e["title"], []).append(e)
    inizi = lambda titolo, conto="mario@alfa.example": sorted(  # noqa: E731
        e["start"] for e in per_titolo.get(titolo, []) if e["account"] == conto)

    # Lo standup è alle 9:00 locali prima e dopo il cambio dell'ora, e una sola volta per giorno
    st = inizi("Standup")
    verifica("serie settimanale: 9:00 locali prima e dopo il cambio dell'ora, una copia per giorno",
             st == ["2026-10-19T09:00:00", "2026-10-21T09:00:00", "2026-10-23T09:00:00", "2026-10-26T09:00:00",
                    "2026-10-28T09:00:00", "2026-10-30T09:00:00", "2026-11-02T09:00:00", "2026-11-04T09:00:00",
                    "2026-11-06T09:00:00"], st)
    scelte = {e["start"][:10]: e for e in per_titolo["Standup"] if e["account"] == "mario@alfa.example"}
    verifica("serie e occorrenze scritte da Outlook: resta quella con luogo e link",
             scelte["2026-10-19"]["location"] == "Sala Rossa" and scelte["2026-10-19"]["online_url"] == TEAMS
             and scelte["2026-10-21"]["location"] == "" and scelte["2026-10-21"]["online_url"] is None,
             (scelte["2026-10-19"]["location"], scelte["2026-10-21"]["location"]))
    verifica("serie a New York: 16:00, 15:00 e 16:00 a Roma (i due cambi dell'ora non coincidono)",
             inizi("Sync New York") == ["2026-10-20T16:00:00", "2026-10-27T15:00:00", "2026-11-03T16:00:00"],
             inizi("Sync New York"))
    verifica("orario UTC: 10:00 a Roma dopo il 25/10 sono le 9:00 UTC", inizi("Call UTC") == ["2026-10-28T09:00:00"],
             inizi("Call UTC"))
    verifica("EXDATE con fuso: il 29/10 non c'è", inizi("Pranzo di squadra") == ["2026-10-22T12:30:00", "2026-11-05T12:30:00"],
             inizi("Pranzo di squadra"))
    verifica("EXDATE in UTC: il 23/10 e il 6/11 non ci sono", inizi("Allineamento") == ["2026-10-30T16:00:00"],
             inizi("Allineamento"))
    verifica("occorrenza annullata: il 23/10 non c'è, il 30/10 e il 6/11 sì",
             inizi("Retro") == ["2026-10-30T16:30:00", "2026-11-06T16:30:00"], inizi("Retro"))
    verifica("occorrenza spostata: il 27/10 è alle 15:00 e con il titolo nuovo, quella delle 11:00 non c'è",
             inizi("Review (spostata)") == ["2026-10-27T15:00:00"]
             and inizi("Review") == ["2026-10-20T11:00:00", "2026-11-03T11:00:00"],
             (inizi("Review (spostata)"), inizi("Review")))
    trimestre = letto["eventi"]["2026-10-01T00:00:00|2026-12-31T00:00:00"]["eventi"]
    doppie = [e for e in trimestre if e["title"] == "Doppia"]
    verifica("due righe con lo stesso titolo e orario: ne resta una, quella col luogo",
             len(doppie) == 1 and doppie[0]["location"] == "Sala Blu", doppie)
    verifica("senza titolo, annullato e cancellato",
             "(senza titolo)" in per_titolo and "Annullata" not in per_titolo and "Cancellata" not in per_titolo,
             sorted(per_titolo))
    verifica("solo la durata: 10:00-10:15", [(e["start"], e["end"]) for e in per_titolo["Sync breve"]]
             == [("2026-10-20T10:00:00", "2026-10-20T10:15:00")], per_titolo.get("Sync breve"))
    verifica("calendario non principale, cancellato e di un'altra app: non si leggono",
             not {"Compleanno ignorato", "Calendario cancellato", "Altra app"} & set(per_titolo), sorted(per_titolo))
    giorni = {e["title"]: (e["start"], e["end"], e["allDay"]) for e in ev if e["allDay"]}
    verifica("giorni interi: a mezzanotte locale, senza fuso",
             giorni.get("Workshop") == ("2026-10-22T00:00:00", "2026-10-23T00:00:00", True)
             and giorni.get("Ferie") == ("2026-10-26T00:00:00", "2026-10-31T00:00:00", True)
             and giorni.get("Compleanno Luca") == ("2026-10-24T00:00:00", "2026-10-25T00:00:00", True), giorni)
    notte = per_titolo["Deploy notturno"][0]
    verifica("a cavallo della mezzanotte e del cambio dell'ora: 22:00 legale, 01:30 legale",
             (notte["start"], notte["end"]) == ("2026-10-24T22:00:00", "2026-10-25T01:30:00"), notte)
    pianif = [e for e in per_titolo["Planning trimestrale"] if e["account"] == "mario@alfa.example"][0]
    verifica("link nel luogo (&amp; letto come &), organizzatore e invitati senza doppioni e in minuscolo",
             pianif["online_url"] == TEAMS + "?x=1&y=2" and pianif["organizer"] == "Anna Rossi"
             and pianif["organizer_email"] == "anna.rossi@alfa.example"
             and pianif["attendees"] == ["anna.rossi@alfa.example", "luca@beta.example", "mario@alfa.example"],
             (pianif["online_url"], pianif["organizer"], pianif["attendees"]))
    google = [e for e in ev if e["account"] == "mario@gmail.example"]
    verifica("i servizi Google del telefono: tipo caldav", google and all(e["tipo"] == "caldav" for e in google),
             [e["tipo"] for e in google])
    verifica("calendario condiviso elencato nelle fonti: l'etichetta porta il suo nome",
             [e["calendar"] for e in per_titolo["Offsite di team"]] == ["mario@beta.example · Shared Team"],
             per_titolo.get("Offsite di team"))
    cal = {c["account"]: c for c in letto["calendari"]["caselle"]}
    verifica("casella che non è sul telefono: errore, e le altre a posto",
             cal["assente@alfa.example"]["stato"] == "errore" and cal["mario@alfa.example"]["stato"] == "ok"
             and letto["eventi"]["2026-10-19T00:00:00|2026-11-09T00:00:00"]["errori"] == [
                 "Assente: La casella assente@alfa.example non e' sul telefono (login da rifare?)"],
             (cal["assente@alfa.example"], letto["eventi"]["2026-10-19T00:00:00|2026-11-09T00:00:00"]["errori"]))
    giorno_cambio = letto["eventi"]["2026-10-25T00:00:00|2026-10-26T00:00:00"]["eventi"]
    verifica("finestra di un giorno (25/10): la notte che finisce all'1:30 e il pranzo, nient'altro",
             sorted(e["title"] for e in giorno_cambio) == ["Deploy notturno", "Pranzo"],
             [e["title"] for e in giorno_cambio])
    con_scarto = letto["eventi"]["2026-11-02T00:00:00+01:00|2026-11-03T00:00:00+01:00"]["eventi"]
    verifica("finestra con lo scarto (+01:00): lo standup delle 9:00 del 2/11",
             "2026-11-02T09:00:00" in [e["start"] for e in con_scarto if e["title"] == "Standup"],
             [(e["title"], e["start"]) for e in con_scarto])


def differenza(a, b):
    import difflib
    ta = json.dumps(a, ensure_ascii=False, sort_keys=True, indent=1).splitlines()
    tb = json.dumps(b, ensure_ascii=False, sort_keys=True, indent=1).splitlines()
    return "\n".join(list(difflib.unified_diff(ta, tb, "registrato", "ottenuto", n=2, lineterm=""))[:80])


def argomento(nome):
    return sys.argv[sys.argv.index(nome) + 1] if nome in sys.argv else None


def main():
    cartella_db = argomento("--db")
    if cartella_db:   # una copia congelata del telefono vero: si registrano le risposte, senza confronti
        with open(argomento("--fonti"), encoding="utf-8") as f:
            fonti = json.load(f)
        with open(argomento("--finestre"), encoding="utf-8") as f:
            finestre = json.load(f)
        letto = leggi_tutto(avvia_agente(cartella_db, fonti), finestre)
        with open(argomento("--uscita"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(letto, f, ensure_ascii=False, sort_keys=True, indent=1)
            f.write("\n")
        print("registrate %d finestre in %s" % (len(finestre), argomento("--uscita")))
        return 0

    cartella = tempfile.mkdtemp()
    costruisci_db(cartella)
    letto = leggi_tutto(avvia_agente(cartella, FONTI), FINESTRE)
    fatti(letto)
    if "--aggiorna" in sys.argv:
        os.makedirs(os.path.dirname(REGISTRATO), exist_ok=True)
        with open(REGISTRATO, "w", encoding="utf-8", newline="\n") as f:
            json.dump(letto, f, ensure_ascii=False, sort_keys=True, indent=1)
            f.write("\n")
        print("riscritto %s (%d eventi nella finestra più lunga)" % (
            REGISTRATO, len(letto["eventi"]["2026-10-01T00:00:00|2026-12-31T00:00:00"]["eventi"])))
    else:
        with open(REGISTRATO, encoding="utf-8") as f:
            registrato = json.load(f)
        uguale = registrato == letto
        verifica("le risposte dell'agente sono quelle registrate in golden/agente.json", uguale,
                 "\n" + differenza(registrato, letto))
    print("TUTTO OK" if not esiti else "%d KO" % esiti)
    return 1 if esiti else 0


if __name__ == "__main__":
    sys.exit(main())
