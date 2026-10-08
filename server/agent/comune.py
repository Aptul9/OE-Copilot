"""Funzioni comuni alle fonti dell'agente: orari locali e link delle riunioni."""
import os
import re
from datetime import date, datetime
from zoneinfo import ZoneInfo

MEETING_URL_RE = re.compile(
    r"https?://(?:[\w.-]*teams\.microsoft\.com|[\w.-]*zoom\.us|meet\.google\.com|"
    r"[\w.-]*webex\.com|meet\.jit\.si|[\w.-]*gotomeeting\.com)/[^\s<>\"']+",
    re.IGNORECASE)


def _fuso_locale():
    # Il fuso vero, con l'ora legale, non lo scostamento di questo momento:
    # con datetime.now().astimezone().tzinfo (+02:00 d'estate) ogni evento
    # dopo il cambio dell'ora usciva un'ora avanti.
    return ZoneInfo(os.environ.get("TZ") or "Europe/Rome")


def _naive_locale(valore):
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
