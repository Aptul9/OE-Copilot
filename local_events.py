"""Eventi aggiunti a mano dentro l'app, calendario "Altro".

Outlook e Google sono in sola lettura: questo è l'unico calendario su cui l'app
scrive. Sta in un file JSON locale, così non tocca nessuna casella di posta e
non richiede permessi. Gli eventi hanno lo stesso formato di quelli letti da
Outlook, così l'interfaccia li tratta allo stesso modo.
"""
import json
import logging
import os
import threading
from datetime import datetime

import config

# Questo e' l'unico calendario su cui l'app SCRIVE: aggiunta e cancellazione
# fanno leggi-modifica-riscrivi, e due chiamate ravvicinate dall'interfaccia
# arrivano su thread diversi. Senza lock una delle due si perde.
_lock = threading.Lock()

log = logging.getLogger(__name__)

ID_CALENDARIO = "local::altro"   # id del "calendario" nell'elenco


def _leggi():
    try:
        with open(config.LOCAL_EVENTS_PATH, "r", encoding="utf-8") as f:
            dati = json.load(f)
        return dati if isinstance(dati, list) else []
    except FileNotFoundError:
        return []          # prima volta: nessun evento personale, e' normale
    except (OSError, json.JSONDecodeError):
        # Qui ci sono dati che esistono solo su questo PC: un file illeggibile
        # non deve passare inosservato.
        log.error("eventi personali: file non leggibile (%s)",
                  config.LOCAL_EVENTS_PATH, exc_info=True)
        return []


def _scrivi(eventi):
    # Salvataggio atomico: sono dati che esistono solo qui, un file troncato
    # significherebbe perderli.
    config.scrivi_json(config.LOCAL_EVENTS_PATH, eventi)


def _nuovo_id(esistenti):
    # Id progressivo semplice: basta che sia unico nel file.
    numeri = [int(e["uid"]) for e in esistenti if str(e.get("uid", "")).isdigit()]
    return str(max(numeri) + 1 if numeri else 1)


def list_calendars(cfg):
    """Voce singola per l'elenco calendari: il calendario locale è sempre presente."""
    disabled = set(cfg.get("disabled_calendars", []))
    return [{
        "id": ID_CALENDARIO,
        "account": config.ACCOUNT_LOCALE,
        "alias": config.ACCOUNT_LOCALE,
        "label": config.ACCOUNT_LOCALE,
        "url": None,
        "count": len(_leggi()),
        "is_ics": False,
        "is_local": True,
        "color": config.color_for_account(cfg, config.ACCOUNT_LOCALE),
        "enabled": ID_CALENDARIO not in disabled,
        # Sta su questo disco: non c'e' un collegamento che possa cadere.
        "stato": "ok",
        "stato_msg": "Locale",
    }]


def add_event(cfg, dati):
    """Aggiunge un evento. dati: title, start, end, allDay, location, notes."""
    titolo = (dati.get("title") or "").strip()
    if not titolo:
        raise ValueError("Il titolo è obbligatorio.")
    inizio = (dati.get("start") or "").strip()
    if not inizio:
        raise ValueError("Manca la data/ora di inizio.")
    try:
        datetime.fromisoformat(inizio)
        if dati.get("end"):
            datetime.fromisoformat(dati["end"])
    except ValueError as e:
        raise ValueError(f"Data non valida: {e}") from e

    with _lock:
        eventi = _leggi()
        eventi.append({
            "uid": _nuovo_id(eventi),
            "title": titolo,
            "start": inizio,
            "end": (dati.get("end") or "").strip() or inizio,
            "allDay": bool(dati.get("allDay")),
            "location": (dati.get("location") or "").strip(),
            "notes": (dati.get("notes") or "").strip(),
        })
        _scrivi(eventi)
    return {"ok": True}


def delete_event(cfg, uid):
    with _lock:
        _scrivi([e for e in _leggi() if str(e.get("uid")) != str(uid)])
    return {"ok": True}


def fetch_events(cfg, start, end):
    """Eventi locali che si sovrappongono all'intervallo (datetime naive)."""
    disabled = set(cfg.get("disabled_calendars", []))
    if ID_CALENDARIO in disabled:
        return []
    colore = config.color_for_account(cfg, config.ACCOUNT_LOCALE)
    out = []
    for e in _leggi():
        try:
            ev_start = datetime.fromisoformat(e["start"])
            ev_end = datetime.fromisoformat(e.get("end") or e["start"])
        except (ValueError, KeyError):
            continue
        if not (ev_end > start and ev_start < end):
            continue
        out.append({
            "id": f"local::{e['uid']}",
            "uid": e["uid"],
            "account": config.ACCOUNT_LOCALE,
            "alias": config.ACCOUNT_LOCALE,
            "calendar": config.ACCOUNT_LOCALE,
            "color": colore,
            "title": e.get("title") or "(senza titolo)",
            "start": e["start"],
            "end": e.get("end") or e["start"],
            "allDay": bool(e.get("allDay")),
            "location": e.get("location") or "",
            "organizer": "",
            "online_url": None,
            "web_link": None,
            "is_local": True,
        })
    return out

