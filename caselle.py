"""Regole comuni alle caselle lette dall'agente remoto: di che azienda è una riunione,
quale copia tenere fra due uguali e che colore darle.

Prima le caselle potevano essere lette anche da Outlook su questo PC (outlook_com): le
regole stanno qui, in un posto solo, e non dipendono da dove arrivano gli eventi.
"""
import json
import logging
import os
import re
import time
from datetime import datetime

import config

log = logging.getLogger(__name__)

# Copia su disco degli eventi gia' letti: permette di mostrare il calendario
# all'istante all'avvio, anche con la sorgente non ancora pronta, invece di una
# griglia vuota. I dati vengono poi sostituiti da quelli veri appena risponde.
EVENTS_CACHE_PATH = os.path.join(config.APP_DIR, "events_cache.json")
MAX_INTERVALLI_IN_CACHE = 12


def parse_bound(s):
    """Legge un estremo dell'intervallo in datetime locale naive.

    FullCalendar manda le date con l'offset ('2026-07-20T00:00:00+02:00'),
    mentre gli eventi sono naive: confrontarli direttamente solleva TypeError.
    Qui l'offset e' autentico, quindi va convertito all'ora locale.
    """
    s = s.strip()
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is not None:
        dt = dt.astimezone().replace(tzinfo=None)
    return dt


# --------------------------------------------------------------------------
# Copia su disco degli eventi
# --------------------------------------------------------------------------
def leggi_cache_eventi():
    try:
        with open(EVENTS_CACHE_PATH, "r", encoding="utf-8") as f:
            dati = json.load(f)
        return dati if isinstance(dati, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def scrivi_cache_eventi(chiave, eventi):
    dati = leggi_cache_eventi()
    dati[chiave] = {"stamp": time.time(), "eventi": eventi}
    if len(dati) > MAX_INTERVALLI_IN_CACHE:
        # Tiene i piu' recenti: navigando tra le settimane le chiavi crescono.
        recenti = sorted(dati.items(), key=lambda kv: kv[1].get("stamp", 0), reverse=True)
        dati = dict(recenti[:MAX_INTERVALLI_IN_CACHE])
    try:
        config.scrivi_json(EVENTS_CACHE_PATH, dati)
    except OSError:
        pass


# --------------------------------------------------------------------------
# Di che azienda e' un appuntamento
# --------------------------------------------------------------------------
# Prefisso che il programma di posta mette davanti al titolo di un invito
# inoltrato, nelle lingue che capitano (FW/Fwd inglese, I italiano, WG tedesco,
# TR francese, RV spagnolo), anche ripetuto.
INOLTRO_RE = re.compile(r"^\s*(?:(?:fwd?|i|wg|tr|rv)\s*:\s*)+", re.I)


def accoda(events, seen, ev, etichetta_calendario, invitato):
    """Aggiunge `ev` a `events`, riconoscendo le copie e scartando i doppioni.

    `seen` è l'indice (chiave -> posizione) che chi legge tiene per tutta la
    lettura. `invitato` è una funzione senza argomenti che dice quale casella
    dell'utente era fra gli invitati: la si chiama solo quando serve.
    """
    # Copia messa qui da un flusso (le riunioni di un'azienda inoltrata, copiate
    # in questa casella): e' di quell'azienda, e il prefisso nel titolo non
    # serve piu' perche' lo dice gia' l'etichetta.
    inoltrata, titolo = config.azienda_da_titolo(ev["title"])
    if inoltrata:
        ev["title"] = titolo
        ev["inoltrata_da"] = inoltrata
    # Se ne' la cartella ne' l'organizzatore dicono di che azienda e', si guarda
    # a quale casella dell'utente era indirizzato l'invito.
    if not (inoltrata
            or config.azienda_da_cartella(etichetta_calendario)
            or config.azienda_da_email(ev.get("organizer_email"))):
        ev["invitato_come"] = invitato()
    # Un invito inoltrato ("FW: ...") e la stessa riunione letta dal calendario
    # condiviso di chi l'ha organizzata sono una riunione sola: il prefisso non
    # conta per riconoscerle. Resta quella col titolo pulito, col link dell'altra
    # se il suo manca.
    titolo_pulito = INOLTRO_RE.sub("", ev["title"]) or ev["title"]
    key = (ev["account"], titolo_pulito, ev["start"], ev["end"])
    if key in seen and not inoltrata and not events[seen[key]].get("inoltrata_da"):
        prima = events[seen[key]]
        if INOLTRO_RE.match(prima["title"]) and titolo_pulito == ev["title"]:
            ev["online_url"] = ev.get("online_url") or prima.get("online_url")
            events[seen[key]] = ev
        else:
            prima["online_url"] = prima.get("online_url") or ev.get("online_url")
        return
    if key in seen:
        # La stessa riunione arrivata sia come invito vero sia come copia del
        # flusso: vince l'invito, che ha il link per partecipare e
        # l'organizzatore reale.
        if events[seen[key]].get("inoltrata_da") and not inoltrata:
            events[seen[key]] = ev
        return
    seen[key] = len(events)
    events.append(ev)


def etichetta(cfg, events):
    """Da' a ogni evento l'azienda (alias) e il colore con cui va mostrato."""
    alias = config.alias_per_account(cfg, {e["account"] for e in events})
    # Colore per azienda ricavato dagli account già colorati: così un evento
    # riclassificato su un'azienda riusa lo stesso colore della casella di
    # quell'azienda, a prescindere da cosa c'è nella finestra corrente. Fallback
    # sui colori degli eventi presenti per aziende non ancora in account_colors.
    colori_noti = cfg.get("account_colors", {})
    alias_noti = config.alias_per_account(cfg, colori_noti.keys())
    colore_azienda = {}
    for acc, col in colori_noti.items():
        colore_azienda.setdefault(alias_noti.get(acc, acc), col)
    for e in events:
        colore_azienda.setdefault(alias.get(e["account"], e["account"]), e["color"])
    # Le aziende inoltrate non hanno una casella: il colore va assegnato qui, se
    # no al primo avvio le loro copie resterebbero del colore della casella.
    for az_inoltrata, d in config.AZIENDE_INOLTRATE.items():
        colore_azienda[az_inoltrata] = config.color_for_account(
            cfg, az_inoltrata, d.get("colore"))
    for e in events:
        e["alias"] = alias.get(e["account"], e["account"])
        if e["account"].lower() in config.CASELLE_DI_COLLEGHI:
            continue   # calendario di un collega: i suoi eventi restano suoi
        # Priorità: la copia di un flusso Power Automate dichiara nel titolo di
        # chi e' (come organizzatore risulti tu, sulla casella che la ospita).
        # Poi una cartella dedicata a un'azienda (es. un calendario
        # sincronizzato via CalDav), perché la copia replicata perde
        # l'organizzatore reale. In mancanza, si guarda chi organizza: un
        # invito finito nel calendario predefinito del profilo ma di
        # un'altra azienda prende comunque quell'azienda.
        az = (e.get("inoltrata_da")
              or config.azienda_da_cartella(e.get("calendar"))
              or config.azienda_da_email(e.get("organizer_email"))
              or config.azienda_da_email(e.get("invitato_come")))
        if az and az != e["alias"]:
            e["alias"] = az
            if az in colore_azienda:
                e["color"] = colore_azienda[az]


# --------------------------------------------------------------------------
# Aziende copiate da un flusso (config.AZIENDE_INOLTRATE)
# --------------------------------------------------------------------------
def calendari_inoltrati(cfg, stato_di):
    """Voci per la barra laterale, una per azienda inoltrata.

    Non corrispondono a una casella (le copie stanno nel calendario di
    un'altra), ma servono lo stesso: interruttore, colore e pallino propri, se
    no l'azienda inoltrata si spegnerebbe solo insieme a quella che ospita le
    copie. `stato_di(azienda)` da' il pallino: dipende da chi legge le caselle.
    """
    disabled = set(cfg.get("disabled_calendars", []))
    out = []
    for azienda, d in config.AZIENDE_INOLTRATE.items():
        cal_id = "inoltro::" + azienda.lower()
        out.append({
            "id": cal_id,
            "account": azienda,
            "alias": azienda,
            "label": "Riunioni " + azienda,
            "count": None,
            "nota": "copiate da Power Automate in " + d["via"],
            "is_inoltro": True,
            "color": config.color_for_account(cfg, azienda, d.get("colore")),
            "enabled": cal_id not in disabled,
            **stato_di(azienda),
        })
    return out


def set_calendar_enabled(cfg, cal_id, enabled):
    disabled = set(cfg.get("disabled_calendars", []))
    if enabled:
        disabled.discard(cal_id)
    else:
        disabled.add(cal_id)
    cfg["disabled_calendars"] = sorted(disabled)
    config.save_config(cfg)
