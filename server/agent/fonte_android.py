"""Calendari letti dal telefono Android virtuale (sulla VM).

L'app Outlook ufficiale gira nel telefono con le caselle collegate; con
"Sincronizza calendari" attivo copia gli appuntamenti nel calendario di
sistema di Android (CalendarProvider, database calendar.db). L'agente non si
collega a nessun servizio: legge una copia di quel database. I login stanno
dentro Outlook sul telefono.

Outlook scrive ogni occorrenza come un appuntamento a se' e, per alcune serie,
anche la serie con la sua regola, le occorrenze modificate e quelle annullate.
La serie si ricostruisce in iCalendar e si espande con recurring_ical_events
(serve per le occorrenze che Outlook non ha ancora scritto una per una), poi si
tolgono i doppioni. Le occorrenze annullate non si
mostrano: Outlook sul PC le tiene come "Annullato: ...", il telefono sa che
sono annullate.
"""
import html
import json
import logging
import os
import re
import shutil
import sqlite3
import tempfile
import time
from datetime import date, datetime, timedelta, timezone
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

import icalendar
import recurring_ical_events

from comune import MEETING_URL_RE, _fuso_locale, _naive_locale, _testo

log = logging.getLogger("android")

DB = "calendar.db"
# Chi scrive nel calendario del telefono (prefissi di Calendars.account_type):
# Outlook per le caselle Microsoft; per i calendari Google i servizi Google del
# telefono (com.google) o DAVx5 (Outlook non esporta gli account Google nel
# calendario di Android).
TIPI_ACCOUNT = ("com.microsoft.office.outlook", "bitfire.at.davdroid", "com.google")
ANNULLATO = 2                                     # Events.STATUS_CANCELED
ORGANIZZATORE = 2                                 # Attendees.RELATIONSHIP_ORGANIZER

# Stato degli accessi sul telefono, scritto dal servizio di sincronizzazione
# (strumenti/rileva_accessi.py): per ogni casella uno stato (ok/avviso/errore) e
# un messaggio. Serve a chiudere il "falso verde": quando un accesso scade il
# calendario Android resta com'era, la lettura riesce lo stesso, e senza questo
# il pallino resterebbe verde su dati fermi.
ACCESSI_PATH = os.environ.get("ACCESSI", "/stato/accessi.json")
_GRAVITA = {"ok": 0, "avviso": 1, "errore": 2}


def _accessi():
    """{email: {"stato", "msg"}} scritto dal rilevatore, o {} se manca."""
    try:
        with open(ACCESSI_PATH, encoding="utf-8") as f:
            return json.load(f).get("caselle") or {}
    except (OSError, ValueError, AttributeError):
        return {}


def _copia_e_apri(cartella):
    """Copia il database con i suoi file di appoggio e lo apre (Android lo tiene aperto)."""
    tmp = tempfile.mkdtemp(prefix="android_")
    for suffisso in ("", "-journal", "-wal", "-shm"):
        sorgente = os.path.join(cartella, DB + suffisso)
        if os.path.exists(sorgente):
            shutil.copy2(sorgente, os.path.join(tmp, DB + suffisso))
    return tmp, sqlite3.connect(os.path.join(tmp, DB))


def _quando(ms, fuso, tutto_il_giorno):
    """Istante di Android (millisecondi UTC + nome del fuso) -> data/ora per iCalendar."""
    utc = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=ms)
    if tutto_il_giorno:
        return utc.date()           # Android salva i giorni interi a mezzanotte UTC
    if not fuso or fuso == "UTC":
        return utc
    try:
        return utc.astimezone(ZoneInfo(fuso))
    except ZoneInfoNotFoundError:
        return utc


_DURATA_RE = re.compile(r"^([+-])?P(?:(\d+)W)?(?:(\d+)D)?T?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$")


def _durata(testo):
    """Durata di Android -> timedelta.

    Android scrive anche "P900S" (senza la T che iCalendar pretende davanti a
    ore, minuti e secondi), che la libreria icalendar rifiuta.
    """
    m = _DURATA_RE.match((testo or "").strip().upper())
    if not m:
        raise ValueError("durata non riconosciuta: %r" % testo)
    segno, settimane, giorni, ore, minuti, secondi = m.groups()
    d = timedelta(weeks=int(settimane or 0), days=int(giorni or 0), hours=int(ore or 0),
                  minutes=int(minuti or 0), seconds=int(secondi or 0))
    return -d if segno == "-" else d


def _pulisci_link(url):
    """Link di riunione come lo si trova nella descrizione -> link usabile.

    Outlook scrive spesso la descrizione in HTML: "&amp;" al posto di "&". E
    quando la mail e' passata dai "safelinks", al link segue la sua copia
    protetta fra parentesi angolari codificate ("...GSj%3Chttps://eur01.
    safelinks...%3E"): l'espressione che riconosce i link si ferma a "<" ma non
    a "%3C", quindi si taglia qui.
    """
    url = html.unescape(url)
    url = re.split(r"%3[Cc](?=https?://)", url)[0]
    return url.rstrip(">).,;\"'")


def _righe(nome, valore):
    """Valore di una colonna di ricorrenza (RRULE, EXDATE...) -> {nome: valore iCalendar}.

    EXDATE/RDATE possono arrivare come "TZID=Europe/Rome;20260801T100000,..."
    o come elenco di date UTC.
    """
    valore = (valore or "").strip()
    if not valore:
        return {}
    if nome in ("EXDATE", "RDATE") and ";" in valore:
        parametri, date_ = valore.rsplit(";", 1)
        riga = "%s;%s:%s" % (nome, parametri, date_)
    else:
        riga = "%s:%s" % (nome, valore)
    finto = icalendar.Event.from_ical("BEGIN:VEVENT\r\n%s\r\nEND:VEVENT\r\n" % riga)
    return dict(finto.items())


class CalendarioAndroid:
    """Una casella di Outlook sul telefono: il suo calendario principale (o quelli indicati)."""

    def __init__(self, id_cal, nome, account, cartella_db, calendari=None):
        self.id, self.nome = id_cal, nome
        self.account = account.strip().lower()
        self.cartella = cartella_db
        self.calendari = [c.strip().lower() for c in (calendari or [])]
        self.stato = {"stato": "avviso", "msg": "Non ancora letto"}
        # Chi tiene aggiornata la casella sul telefono: "outlook" o "caldav"
        # (DAVx5 o servizi Google: il nome resta "caldav" perché l'app sul PC lo
        # conosce). All'app serve per sapere se è una casella Microsoft.
        self.tipo = None
        self.conteggio = None             # appuntamenti presenti sul telefono

    # -- lettura -------------------------------------------------------------
    def _leggi(self):
        if not os.path.exists(os.path.join(self.cartella, DB)):
            raise RuntimeError("calendario del telefono non trovato")
        ultimo = None
        for _ in range(2):   # una copia presa a meta' scrittura si ritenta
            tmp, c = _copia_e_apri(self.cartella)
            try:
                return self._estrai(c)
            except sqlite3.DatabaseError as e:
                ultimo = e
                time.sleep(1)
            finally:
                c.close()
                shutil.rmtree(tmp, ignore_errors=True)
        raise ultimo

    def _estrai(self, c):
        cal, sincronizzati = {}, {}
        for _id, conto, tipo, nome, primario, cancellato, sincr in c.execute(
                "select _id, account_name, account_type, calendar_displayName, isPrimary, deleted, "
                "sync_events from Calendars"):
            if (conto or "").lower() != self.account or not (tipo or "").startswith(TIPI_ACCOUNT):
                continue
            if cancellato:
                continue
            self.tipo = "caldav" if tipo.startswith(("bitfire", "com.google")) else "outlook"
            if sincr:
                sincronizzati[_id] = nome or "Calendario"
            scelto = ((nome or "").lower() in self.calendari) if self.calendari else bool(primario)
            if scelto:
                cal[_id] = nome or "Calendario"
        # DAVx5 non segna un calendario principale: senza scelta esplicita si
        # prendono quelli che sincronizza (in DAVx5 si attivano uno per uno).
        if not cal and not self.calendari:
            cal = sincronizzati
        if not cal:
            return None
        segnaposti = ",".join("?" * len(cal))
        eventi = c.execute(
            "select _id, _sync_id, calendar_id, title, eventLocation, description, eventStatus, "
            "dtstart, dtend, eventTimezone, eventEndTimezone, duration, allDay, rrule, rdate, "
            "exrule, exdate, original_id, original_sync_id, originalInstanceTime, originalAllDay, "
            "organizer from Events where deleted = 0 and calendar_id in (%s)" % segnaposti,
            list(cal)).fetchall()
        persone = {}
        for evento, nome, email, relazione in c.execute(
                "select event_id, attendeeName, attendeeEmail, attendeeRelationship from Attendees "
                "where event_id in (select _id from Events where calendar_id in (%s))" % segnaposti,
                list(cal)):
            p = persone.setdefault(evento, {"invitati": [], "organizzatore": None})
            if email:
                p["invitati"].append(email.lower())
            if relazione == ORGANIZZATORE:
                p["organizzatore"] = nome or email
        return cal, eventi, persone

    # -- ricostruzione in iCalendar ---------------------------------------------
    def _calendario_ical(self, cal, eventi, persone):
        vcal = icalendar.Calendar()
        vcal.add("prodid", "-//OE Copilot//android//IT")
        vcal.add("version", "2.0")
        dettagli = {}                     # UID iCalendar -> campi non iCalendar
        per_sync = {r[1]: r[0] for r in eventi if r[1]}
        annullate = {}                    # id della serie -> [occorrenze annullate]
        for (_id, sync_id, cal_id, titolo, luogo, descrizione, stato, ini, fin, tz, tz_fine,
             durata, giorno, rrule, rdate, exrule, exdate, orig_id, orig_sync, orig_ms,
             orig_giorno, organizzatore) in eventi:
            serie = orig_id or per_sync.get(orig_sync)
            if serie and stato == ANNULLATO and orig_ms is not None:
                annullate.setdefault(serie, []).append((orig_ms, orig_giorno))
                continue
            if stato == ANNULLATO or ini is None:
                continue
            uid = "a%s" % (serie or _id)
            ev = icalendar.Event()
            ev.add("uid", uid)
            ev.add("summary", titolo or "")
            ev.add("dtstart", _quando(ini, tz, giorno))
            if fin is not None:
                ev.add("dtend", _quando(fin, tz_fine or tz, giorno))
            elif durata:
                ev.add("duration", _durata(durata))
            if serie and orig_ms is not None:
                ev.add("recurrence-id", _quando(orig_ms, tz, orig_giorno))
            elif not serie:
                for nome, valore in (("RRULE", rrule), ("RDATE", rdate), ("EXRULE", exrule),
                                     ("EXDATE", exdate)):
                    for k, v in _righe(nome, valore).items():
                        ev[k] = v
            if luogo:
                ev.add("location", luogo)
            if descrizione:
                ev.add("description", descrizione)
            vcal.add_component(ev)
            p = persone.get(_id, {})
            dettagli.setdefault(uid, {
                "calendario": cal.get(cal_id, ""),
                "organizzatore_email": (organizzatore or "").lower(),
                "organizzatore": p.get("organizzatore") or organizzatore or "",
                "invitati": sorted(set(p.get("invitati", []))),
            })
            if serie or rrule or rdate:
                dettagli[uid]["serie"] = True
        # Le occorrenze annullate di una serie diventano EXDATE della serie.
        for comp in vcal.walk("VEVENT"):
            serie = str(comp.get("UID"))[1:]
            if comp.get("RRULE") and serie.isdigit() and int(serie) in annullate:
                fuso = comp.get("DTSTART").dt
                for ms, giorno in annullate[int(serie)]:
                    tz = getattr(fuso, "tzinfo", None)
                    quando = _quando(ms, getattr(tz, "key", None) if tz else None, giorno)
                    comp.add("exdate", quando)
        return vcal, dettagli

    # -- interfaccia per l'agente ----------------------------------------------
    def _leggi_e_segna(self):
        """Legge il telefono e aggiorna lo stato. -> (dati grezzi o None, errori)."""
        try:
            grezzi = self._leggi()
        except Exception as e:
            log.warning("%s: lettura non riuscita (%s)", self.nome, e)
            self.stato = {"stato": "errore", "msg": "Calendario del telefono non leggibile (%s)" % e}
            return None, ["%s: %s" % (self.nome, e)]
        if grezzi is None:
            msg = "La casella %s non e' sul telefono (login da rifare?)" % self.account
            self.stato = {"stato": "errore", "msg": msg}
            return None, ["%s: %s" % (self.nome, msg)]
        self.conteggio = len(grezzi[1])
        # Si dice solo cosa si è verificato davvero: la lettura è riuscita. L'ora di
        # scrittura di calendar.db non dice niente di Outlook (la muove ogni 2 minuti
        # la sincronizzazione Google): la freschezza di ogni casella la misura
        # strumenti/rileva_accessi.py.
        self.stato = {"stato": "ok", "msg": "Letto dal telefono alle %s" % time.strftime("%H:%M")}
        self._applica_stato_accesso()
        return grezzi, []

    def _applica_stato_accesso(self):
        """Se l'accesso alla casella sul telefono non e' a posto, lo dice il pallino.

        La lettura del calendario riesce anche con il login scaduto (sono dati
        gia' sul telefono), quindi senza questo controllo il pallino resterebbe
        verde mentre il calendario e' fermo a prima della scadenza. Il verdetto
        (ok/avviso/errore + messaggio) arriva dal rilevatore, che guarda solo la
        bandierina di Outlook e da quanto DAVx5 non sincronizza: nessun token
        entra mai qui. Si applica solo se peggiora lo stato della lettura.
        """
        v = _accessi().get(self.account)
        if not v:
            return
        stato = v.get("stato") or "ok"
        if _GRAVITA.get(stato, 0) > _GRAVITA.get(self.stato.get("stato"), 0):
            self.stato = {"stato": stato, "msg": v.get("msg") or self.stato.get("msg", "")}

    def descrizione(self):
        """La casella come la vede l'app: chi e', com'e' andata l'ultima lettura."""
        return {"id": self.id, "account": self.account, "nome": self.nome, "tipo": self.tipo,
                "eventi": self.conteggio, **self.stato}

    def riepilogo(self):
        """Come descrizione(), ma dopo una lettura fresca (per l'elenco dei calendari)."""
        self._leggi_e_segna()
        return self.descrizione()

    def eventi(self, inizio, fine):
        """Eventi fra due datetime locali naive. -> (eventi, errori)."""
        grezzi, errori = self._leggi_e_segna()
        if grezzi is None:
            return [], errori
        cal, eventi, persone = grezzi
        vcal, dettagli = self._calendario_ical(cal, eventi, persone)
        fuso = _fuso_locale()
        trovati = recurring_ical_events.of(vcal).between(
            inizio.replace(tzinfo=fuso), fine.replace(tzinfo=fuso))
        candidati = {}
        for ev in trovati:
            dtstart = ev.get("DTSTART")
            ini = _naive_locale(dtstart.dt if dtstart else None)
            if ini is None:
                continue
            dtend = ev.get("DTEND")
            fin = _naive_locale(dtend.dt if dtend else None)
            if fin is None and ev.get("DURATION"):
                fin = ini + ev.get("DURATION").dt
            fin = fin or ini
            tutto_giorno = isinstance(dtstart.dt, date) and not isinstance(dtstart.dt, datetime)
            uid = _testo(ev, "UID")
            d = dettagli.get(uid, {})
            descrizione, luogo = _testo(ev, "DESCRIPTION"), _testo(ev, "LOCATION")
            link = MEETING_URL_RE.search(luogo) or MEETING_URL_RE.search(descrizione)
            e = {
                "id": "%s|%s|%s" % (self.id, uid, ini.isoformat()),
                "account": self.account, "alias": self.nome,
                "calendar": "%s · %s" % (self.account, d.get("calendario", "")),
                "title": _testo(ev, "SUMMARY") or "(senza titolo)",
                "start": ini.isoformat(), "end": fin.isoformat(),
                "allDay": bool(tutto_giorno), "location": luogo,
                "organizer": d.get("organizzatore", ""),
                "organizer_email": d.get("organizzatore_email", ""),
                "attendees": d.get("invitati", []),
                "online_url": _pulisci_link(link.group(0)) if link else None,
                "web_link": None, "fonte": "android", "tipo": self.tipo,
            }
            # Outlook scrive una serie due volte: la serie con la sua regola E ogni
            # occorrenza come appuntamento a se'. Espandendo la serie le occorrenze
            # raddoppierebbero: per titolo, inizio e fine (la regola dei doppioni
            # dell'app sul PC) si tiene una copia sola, la piu' completa. Prima
            # quella scritta da Outlook per l'occorrenza (ha il luogo e il link di
            # quel giorno) rispetto a quella ricavata dalla regola, poi quella con
            # link, luogo e organizzatore.
            peso = (not d.get("serie"), bool(e["online_url"]), bool(luogo),
                    bool(e["organizer_email"]))
            chiave = (e["title"], ini, fin)
            if chiave not in candidati or peso > candidati[chiave][0]:
                candidati[chiave] = (peso, e)
        out = sorted((e for _, e in candidati.values()), key=lambda e: (e["start"], e["title"]))
        return out, []
