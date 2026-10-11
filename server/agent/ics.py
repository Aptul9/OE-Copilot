"""Calendari iCalendar (ICS) letti per conto dell'app: validazione ed espansione delle ricorrenze.

L'app (il gateway) scarica da sé il testo dei calendari da link, con la sua copia e il suo ritardo, e lo manda qui:
le ricorrenze (RRULE, RDATE, EXDATE, RECURRENCE-ID, fusi, cambio dell'ora) le espande la stessa libreria che
legge il telefono (recurring_ical_events), una sola per tutte le sorgenti. Funzioni pure: nessuna rete, nessun file.
"""
from datetime import date, datetime

import icalendar
import recurring_ical_events

from comune import MEETING_URL_RE


def _naive_locale(valore):
    """Porta qualunque data/ora ICS a datetime locale senza fuso."""
    if isinstance(valore, datetime):
        if valore.tzinfo is not None:
            # Senza argomenti: lo scarto che il sistema ha in QUELLA data (non quello di oggi)
            return valore.astimezone().replace(tzinfo=None)
        return valore
    if isinstance(valore, date):
        return datetime(valore.year, valore.month, valore.day)
    return None


def _testo(componente, chiave):
    valore = componente.get(chiave)
    return str(valore) if valore is not None else ""


def valida(testo):
    """Il nome che il calendario dichiara (X-WR-CALNAME, o "") se il testo è un calendario iCal; ValueError o TypeError se no."""
    cal = icalendar.Calendar.from_ical(testo)
    return str(cal.get("X-WR-CALNAME") or "")


def eventi(testo, inizio, fine):
    """Gli eventi del calendario nell'intervallo, ricorrenze espanse. `inizio` e `fine` sono datetime locali senza fuso.

    Ogni voce: uid, inizio e fine (testo ISO, ora locale senza fuso), tutto_giorno, titolo, luogo, organizzatore, link.
    """
    cal = icalendar.Calendar.from_ical(testo)
    trovati = recurring_ical_events.of(cal).between(inizio.astimezone(), fine.astimezone())
    out = []
    for ev in trovati:
        ini = _naive_locale(ev.get("DTSTART").dt if ev.get("DTSTART") else None)
        if ini is None:
            continue
        fin = _naive_locale(ev.get("DTEND").dt if ev.get("DTEND") else None) or ini
        tutto_giorno = isinstance(ev.get("DTSTART").dt, date) and not isinstance(ev.get("DTSTART").dt, datetime)
        descrizione = _testo(ev, "DESCRIPTION")
        luogo = _testo(ev, "LOCATION")
        link = MEETING_URL_RE.search(luogo) or MEETING_URL_RE.search(descrizione)
        out.append({
            "uid": _testo(ev, "UID"),
            "inizio": ini.isoformat(), "fine": fin.isoformat(),
            "tutto_giorno": bool(tutto_giorno),
            "titolo": _testo(ev, "SUMMARY"),
            "luogo": luogo,
            "organizzatore": _testo(ev, "ORGANIZER").replace("mailto:", ""),
            "link": link.group(0).rstrip(">).,;\"'") if link else None,
        })
    return out
