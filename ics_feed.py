"""Calendari sottoscritti tramite link ICS (Google Calendar, Apple, ecc.).

Serve per le caselle che Outlook non porta: gli account IMAP sincronizzano solo
la posta, mai il calendario. Google pubblica per ogni calendario un "indirizzo
segreto in formato iCal" che si puo' leggere senza credenziali e senza il
consenso di un amministratore -- l'unico modo praticabile visto che i tenant
aziendali bloccano il consenso alle app di terze parti.

Il link e' di fatto una chiave: chi ce l'ha legge il calendario. Resta quindi
solo in config.json su questo PC, come le altre impostazioni.
"""
import hashlib
import logging
import os
import re
import time
from datetime import date, datetime, timedelta

import icalendar
import recurring_ical_events
import requests

import config
# Stessa estrazione del link riunione usata per gli eventi di Outlook: tenerla
# in un posto solo evita che le due strade divergano.
from outlook_com import MEETING_URL_RE

log = logging.getLogger(__name__)

CACHE_DIR = os.path.join(config.APP_DIR, "ics_cache")
# Google rigenera l'ICS ogni qualche ora: scaricarlo a ogni cambio di vista
# sarebbe inutile e lento.
TTL_SECONDS = 1800


def _feed_id(url):
    return "ics_" + hashlib.sha1(url.encode("utf-8")).hexdigest()[:12]


def normalizza_url(url):
    """Accetta anche i link webcal:// che usano diversi calendari."""
    url = (url or "").strip()
    if url.lower().startswith("webcal://"):
        url = "https://" + url[len("webcal://"):]
    return url


# Esito dell'ultima lettura per feed, per accendere il pallino di stato
# nell'elenco calendari: un feed irraggiungibile deve vedersi, non sparire.
_stato_feed = {}


def list_feeds(cfg):
    disabled = set(cfg.get("disabled_calendars", []))
    out = []
    # Le festività compaiono in elenco come gli altri: sono un calendario a
    # tutti gli effetti e chi non le vuole vedere deve poterle spegnere.
    for f in [_feed_festivita()] + list(cfg.get("ics_calendars", [])):
        out.append({
            "id": f["id"],
            "account": f.get("name") or f["url"],
            "label": f.get("name") or f["url"],
            "url": f["url"],
            "count": None,          # un ICS non ha un conteggio a colpo d'occhio
            "is_ics": True,
            "color": (config.COLORE_FESTIVITA if f.get("builtin")
                      else config.color_for_account(cfg, f.get("name") or f["url"])),
            "enabled": f["id"] not in disabled,
            "builtin": bool(f.get("builtin")),
            # Verde solo se una lettura e' andata a buon fine davvero.
            "stato": _stato_feed.get(f["id"], {}).get("stato", "avviso"),
            "stato_msg": _stato_feed.get(f["id"], {}).get("msg", "Non ancora letto"),
        })
    return out


def add_feed(cfg, name, url):
    """Aggiunge un calendario da link. Verifica subito che sia leggibile."""
    url = normalizza_url(url)
    if not url.lower().startswith(("http://", "https://")):
        raise ValueError("Il link deve iniziare con https:// (o webcal://).")

    try:
        testo = _scarica(url)
    except requests.RequestException as e:
        raise ValueError(f"Il link non risponde: {e}") from e
    try:
        cal = icalendar.Calendar.from_ical(testo)
    except (ValueError, TypeError) as e:
        raise ValueError(
            "Il link risponde ma non contiene un calendario iCal valido. "
            "Assicurati di aver copiato l'indirizzo segreto in formato iCal."
        ) from e

    fid = _feed_id(url)
    feeds = cfg.setdefault("ics_calendars", [])
    if any(f["id"] == fid for f in feeds):
        raise ValueError("Questo calendario è già presente.")
    nome = (name or "").strip() or str(cal.get("X-WR-CALNAME") or "Calendario ICS")
    feeds.append({"id": fid, "name": nome, "url": url})
    config.save_config(cfg)
    _salva_cache(fid, testo)
    return {"id": fid, "name": nome, "url": url}


def remove_feed(cfg, feed_id):
    feeds = cfg.get("ics_calendars", [])
    cfg["ics_calendars"] = [f for f in feeds if f["id"] != feed_id]
    disabled = set(cfg.get("disabled_calendars", []))
    disabled.discard(feed_id)
    cfg["disabled_calendars"] = sorted(disabled)
    config.save_config(cfg)
    try:
        os.remove(_percorso_cache(feed_id))
    except OSError:
        pass


# --------------------------------------------------------------------------
# Scarico e cache
# --------------------------------------------------------------------------
def _percorso_cache(feed_id):
    # I due punti non sono ammessi nei nomi file su Windows, e l'identificativo
    # del calendario festività li contiene ("builtin::festivita-it"): la copia
    # locale non veniva mai scritta e il file si riscaricava a ogni lettura.
    # L'errore restava invisibile perché il modulo non aveva un logger.
    nome = re.sub(r"[^A-Za-z0-9._-]", "_", str(feed_id))
    return os.path.join(CACHE_DIR, nome + ".ics")


def _salva_cache(feed_id, testo):
    try:
        os.makedirs(CACHE_DIR, exist_ok=True)
        with open(_percorso_cache(feed_id), "w", encoding="utf-8") as f:
            f.write(testo)
    except OSError:
        # La copia locale e' un'ottimizzazione, ma se non si scrive ogni lettura
        # ripartira' dalla rete: meglio saperlo.
        log.warning("ics: copia locale di %s non scritta", feed_id, exc_info=True)


def _scarica(url):
    r = requests.get(url, timeout=25, headers={"User-Agent": "CalendarHub/1.0"})
    r.raise_for_status()
    return r.text


def _testo_calendario(feed_id, url):
    """Testo ICS, dalla cache se fresca. Ritorna (testo, errore_soft)."""
    percorso = _percorso_cache(feed_id)
    fresca = False
    try:
        fresca = (time.time() - os.path.getmtime(percorso)) < TTL_SECONDS
    except OSError:
        pass
    if fresca:
        try:
            with open(percorso, "r", encoding="utf-8") as f:
                return f.read(), None
        except OSError:
            pass
    try:
        testo = _scarica(url)
        _salva_cache(feed_id, testo)
        return testo, None
    except requests.RequestException as e:
        # Rete assente: meglio dati vecchi che nessun dato.
        try:
            with open(percorso, "r", encoding="utf-8") as f:
                return f.read(), f"aggiornamento non riuscito ({e})"
        except OSError:
            return None, str(e)


# --------------------------------------------------------------------------
# Lettura eventi
# --------------------------------------------------------------------------
def _fuso_locale():
    return datetime.now().astimezone().tzinfo


def _naive_locale(valore):
    """Porta qualunque data/ora ICS a datetime locale senza fuso."""
    if isinstance(valore, datetime):
        if valore.tzinfo is not None:
            return valore.astimezone(_fuso_locale()).replace(tzinfo=None)
        return valore
    if isinstance(valore, date):
        return datetime(valore.year, valore.month, valore.day)
    return None


def _testo(componente, chiave):
    valore = componente.get(chiave)
    return str(valore) if valore is not None else ""


def _feed_festivita():
    """Il calendario festività italiane, sempre presente e non modificabile."""
    return {"id": config.FESTIVITA_ID, "name": config.FESTIVITA_NOME,
            "url": config.FESTIVITA_URL, "builtin": True}


def fetch_events(cfg, start, end):
    """Eventi dei calendari ICS attivi nell'intervallo. Ritorna (eventi, errori).

    start/end sono datetime locali naive, come per la lettura da Outlook.
    """
    # Le festività italiane sono un feed fisso, oltre a quelli aggiunti dall'utente.
    feeds = [_feed_festivita()] + list(cfg.get("ics_calendars", []))
    disabled = set(cfg.get("disabled_calendars", []))
    fuso = _fuso_locale()
    eventi, errori = [], []

    for feed in feeds:
        builtin = feed.get("builtin")
        spento = feed["id"] in disabled
        if spento and not builtin:
            continue
        # Le festività si leggono anche da spente (costa nulla: c'e' la copia
        # locale) ma partono marcate "nascosto": l'interfaccia le scarta sia
        # dalla griglia sia dalla riga sotto il giorno, com'e' stato chiesto.
        nome = feed.get("name") or feed["url"]
        testo, errore = _testo_calendario(feed["id"], feed["url"])
        # Se il download fallisce ma la copia locale c'e', il calendario si vede
        # comunque: e' un avviso, non un guasto (rosso solo se non c'e' nulla).
        if not errore:
            _stato_feed[feed["id"]] = {"stato": "ok", "msg": "Aggiornato"}
        else:
            _stato_feed[feed["id"]] = {
                "stato": "avviso" if testo else "errore", "msg": errore}
        if errore:
            errori.append(f"{nome}: {errore}")
        if not testo:
            continue
        try:
            cal = icalendar.Calendar.from_ical(testo)
            # La libreria espande le ricorrenze (RRULE, eccezioni comprese):
            # farlo a mano sarebbe una fonte inesauribile di bachi.
            trovati = recurring_ical_events.of(cal).between(
                start.replace(tzinfo=fuso), end.replace(tzinfo=fuso))
        except Exception as e:  # ICS malformato: non deve far cadere il resto
            errori.append(f"{nome}: calendario illeggibile ({e})")
            continue

        colore = config.COLORE_FESTIVITA if builtin else config.color_for_account(cfg, nome)
        for ev in trovati:
            inizio = _naive_locale(ev.get("DTSTART").dt if ev.get("DTSTART") else None)
            if inizio is None:
                continue
            fine_ev = _naive_locale(ev.get("DTEND").dt if ev.get("DTEND") else None) or inizio
            tutto_giorno = isinstance(ev.get("DTSTART").dt, date) and not isinstance(
                ev.get("DTSTART").dt, datetime)
            descrizione = _testo(ev, "DESCRIPTION")
            luogo = _testo(ev, "LOCATION")
            link = MEETING_URL_RE.search(luogo) or MEETING_URL_RE.search(descrizione)
            organizzatore = _testo(ev, "ORGANIZER").replace("mailto:", "")
            eventi.append({
                "id": f"{feed['id']}|{_testo(ev, 'UID')}|{inizio.isoformat()}",
                "account": nome,
                "alias": nome,   # per gli ICS il nome scelto è già l'etichetta
                "calendar": nome,
                "color": colore,
                "title": _testo(ev, "SUMMARY") or "(senza titolo)",
                "start": inizio.isoformat(),
                "end": fine_ev.isoformat(),
                "allDay": bool(tutto_giorno),
                "location": luogo,
                "organizer": organizzatore,
                "online_url": link.group(0).rstrip(">).,;\"'") if link else None,
                "web_link": None,
                "nascosto": bool(spento),
            })
    return eventi, errori

