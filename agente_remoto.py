"""Caselle lette dall'agente sul server, con una richiesta HTTP e la chiave dell'API.

L'agente tiene i login, legge le caselle e le espone con una piccola API, in ascolto
solo sulla rete di Docker del server (e sul localhost del server). Il servizio ui
(server/ui/server.py) la raggiunge all'indirizzo che gli dà OE_AGENTE_URL, con la
chiave in OE_AGENTE_CHIAVE: niente è esposto su internet.

Le regole su aziende, doppioni e colori sono quelle comuni (caselle.py).
"""
import json
import logging
import os
from datetime import datetime

import requests

import caselle
import config

log = logging.getLogger(__name__)

# L'agente è un altro container della stessa rete: indirizzo e chiave arrivano dall'ambiente.
URL_DIRETTO = os.environ.get("OE_AGENTE_URL", "").rstrip("/")
CHIAVE_DIRETTA = os.environ.get("OE_AGENTE_CHIAVE", "").strip()
CASELLE_NOTE_PATH = os.path.join(config.DATA_DIR, "agente_caselle.json")

# Oltre questi secondi senza segni di vita dal servizio che aggiorna le caselle
# sul server, i dati non si possono piu' dare per freschi: prima giallo, poi
# rosso (rosso vuol dire "devi intervenire").
SERVIZIO_LENTO_S = 180
SERVIZIO_FERMO_S = 900


class AgenteNonDisponibile(RuntimeError):
    """L'agente non e' raggiungibile, o rifiuta la richiesta."""


def attivo():
    """Vero se l'agente è configurato: OE_AGENTE_URL."""
    return bool(URL_DIRETTO)


# --------------------------------------------------------------------------
# Chiamate all'agente
# --------------------------------------------------------------------------
_sessione = requests.Session()
# L'agente sta sulla rete di Docker: un proxy configurato nel sistema non c'entra e lo romperebbe.
_sessione.trust_env = False


def _chiedi(percorso, parametri=None, attesa=20):
    """GET all'agente. -> dati JSON, o AgenteNonDisponibile."""
    if not attivo():
        raise AgenteNonDisponibile("Agente non configurato: manca OE_AGENTE_URL")
    try:
        r = _sessione.get(URL_DIRETTO + percorso, params=parametri, timeout=(3, attesa),
                          headers={"Authorization": "Bearer " + CHIAVE_DIRETTA})
    except requests.RequestException as e:
        log.warning("agente remoto: richiesta fallita (%s)", e)
        raise AgenteNonDisponibile(
            "Il server dei calendari non risponde (%s)" % e.__class__.__name__) from e
    if r.status_code == 401:
        raise AgenteNonDisponibile("Il server dei calendari rifiuta la chiave: va reimpostata")
    if r.status_code != 200:
        raise AgenteNonDisponibile("Il server dei calendari ha risposto %d" % r.status_code)
    try:
        return r.json()
    except ValueError as e:
        raise AgenteNonDisponibile("Risposta del server dei calendari non valida") from e


# --------------------------------------------------------------------------
# Stato: il pallino di ogni casella
# --------------------------------------------------------------------------
_GRAVITA = {"ok": 0, "avviso": 1, "errore": 2}
# Stato di ogni casella all'ultima lettura, riuscita o no (indirizzo ->
# {"stato", "msg"}): serve al pallino delle aziende inoltrate, le cui copie
# stanno in una di queste caselle.
_ultimo_stato = {}
# Le caselle dell'ultima risposta dell'agente (indirizzo e tipo), tenute anche
# su disco: quando l'agente non risponde -- tipico: l'app parte prima della
# rete -- la barra laterale mostra comunque le aziende, in rosso, invece di
# svuotarsi proprio quando c'e' un guasto da far vedere.
_caselle_note = None


def _note():
    global _caselle_note
    if _caselle_note is None:
        try:
            with open(CASELLE_NOTE_PATH, "r", encoding="utf-8") as f:
                _caselle_note = [c for c in json.load(f)
                                 if c.get("account") and not config.account_nascosto(c["account"])]
        except (OSError, ValueError, TypeError, AttributeError):
            _caselle_note = []
    return _caselle_note


def _ricorda(visibili):
    global _caselle_note
    nuove = [{"account": c["account"].lower(), "tipo": c.get("tipo")} for c in visibili]
    if nuove == _note():
        return
    _caselle_note = nuove
    try:
        config.scrivi_json(CASELLE_NOTE_PATH, nuove)
    except OSError:
        log.warning("agente remoto: elenco delle caselle non salvato", exc_info=True)


def _tutte_ferme(errore):
    """L'agente non risponde: ogni casella nota e' in errore. -> i loro stati"""
    global _ultimo_stato
    _ultimo_stato = {c["account"]: {"stato": "errore", "msg": str(errore)} for c in _note()}
    return _ultimo_stato


def _guasto_servizio(servizio, tipo):
    """(stato, messaggio) se il meccanismo che aggiorna le caselle non gira bene.

    L'agente legge una copia che altri programmi tengono aggiornata sul server:
    se quelli si fermano, la lettura riesce lo stesso ma i dati sono fermi.
    Un pallino verde in quel caso mentirebbe. Se Outlook stesso non risponde o non
    riceve più le notifiche lo dice il rilevatore del server, casella per casella
    (strumenti/rileva_accessi.py): "Outlook non è davanti" non è più un guasto.
    """
    if not servizio:
        return "avviso", "Dal server non arriva il segno di vita dell'aggiornamento"
    if not servizio.get("telefono", True):
        return "errore", "Sul server il lettore delle caselle è spento"
    eta = int(servizio.get("eta_s") or 0)
    if eta > SERVIZIO_FERMO_S:
        return "errore", "Sul server l'aggiornamento è fermo da %d minuti" % (eta // 60)
    if eta > SERVIZIO_LENTO_S:
        return "avviso", "Sul server l'aggiornamento è in ritardo di %d minuti" % (eta // 60)
    if servizio.get("pausa"):
        return "avviso", "Sul server l'aggiornamento è in pausa (accesso in corso?)"
    return None


def _stato_casella(casella, servizio):
    stato = casella.get("stato") or "ok"
    msg = casella.get("msg") or ""
    if stato == "ok":
        guasto = _guasto_servizio(servizio, casella.get("tipo"))
        if guasto:
            stato, msg = guasto[0], " · ".join(x for x in (guasto[1], msg) if x)
    return {"stato": stato, "msg": msg}


def _peggiore(a, b):
    """Di due stati della stessa azienda tiene il piu' grave, con tutti i messaggi."""
    if a is None:
        return b
    grave = b if _GRAVITA.get(b["stato"], 0) > _GRAVITA.get(a["stato"], 0) else a
    return {"stato": grave["stato"],
            "msg": " · ".join(dict.fromkeys(x for x in (a["msg"], b["msg"]) if x))}


def _id_calendario(indirizzo):
    return "remoto::" + indirizzo.lower()


def _caselle_visibili(dati):
    return [c for c in (dati.get("caselle") or [])
            if c.get("account") and not config.account_nascosto(c["account"])]


def stato_inoltro(azienda):
    """Pallino di un'azienda inoltrata, quando le caselle si leggono da remoto.

    Le copie stanno nel calendario di un'altra casella: se quella si legge, le
    copie arrivano. Che il flusso che le crea sia vivo, da qui non si puo'
    sapere (lo direbbe la posta inoltrata, che l'agente non legge): lo si dice.
    """
    via = str(config.AZIENDE_INOLTRATE.get(azienda, {}).get("via") or "").lower()
    ospite = _ultimo_stato.get(via)
    if ospite is None:
        return {"stato": "avviso", "stato_msg": "Non ancora letto"}
    if ospite["stato"] != "ok":
        return {"stato": ospite["stato"], "stato_msg": ospite["msg"]}
    return {"stato": "ok", "stato_msg": "Copie lette dal calendario di %s; flusso non "
            "verificabile da remoto" % via}


def calendari_inoltrati(cfg):
    return caselle.calendari_inoltrati(cfg, stato_inoltro)


# --------------------------------------------------------------------------
# Calendari ed eventi
# --------------------------------------------------------------------------
def list_calendars(cfg, force=False):
    """Una voce per casella letta dall'agente, con colore, stato e interruttore.

    Se l'agente non risponde si da' l'ultimo elenco riuscito, con ogni casella
    in errore. Con force=True, o se un elenco non c'e' ancora, si solleva
    AgenteNonDisponibile.
    """
    global _ultimo_stato
    try:
        dati = _chiedi("/api/calendari")
    except AgenteNonDisponibile as e:
        if force or not _note():
            raise
        caselle_lette, stati = _note(), _tutte_ferme(e)
    else:
        caselle_lette = _caselle_visibili(dati)
        _ricorda(caselle_lette)
        servizio = dati.get("servizio")
        stati = {c["account"].lower(): _stato_casella(c, servizio) for c in caselle_lette}
        _ultimo_stato = stati
    disabled = set(cfg.get("disabled_calendars", []))
    out = []
    for c in caselle_lette:
        account = c["account"].lower()
        cal_id = _id_calendario(account)
        out.append({
            "id": cal_id,
            "account": account,
            "label": f"{account} · Calendario",
            "count": c.get("eventi") or 0,
            "color": config.color_for_account(cfg, account),
            "enabled": cal_id not in disabled,
            "stato": stati[account]["stato"],
            "stato_msg": stati[account]["msg"],
        })
    alias = config.alias_per_account(cfg, [c["account"] for c in out])
    for c in out:
        c["alias"] = alias.get(c["account"], c["account"])
    return out


def _senza_agente(cfg, errore, chiave_cache, allow_cache, meta):
    """Risposta quando l'agente non si raggiunge: dati salvati e pallini rossi."""
    meta["outlook_starting"] = True      # l'interfaccia mostra i dati salvati e riprova
    meta["avviso_sorgente"] = "Il server dei calendari non risponde"
    stati = _tutte_ferme(errore)
    alias = config.alias_per_account(cfg, list(stati))
    disabled = set(cfg.get("disabled_calendars", []))
    meta["aziende"], meta["stato_aziende"] = {}, {}
    for account, stato in stati.items():
        azienda = alias[account]
        meta["aziende"][azienda] = (meta["aziende"].get(azienda, False)
                                    or _id_calendario(account) not in disabled)
        meta["stato_aziende"][azienda] = stato
    if allow_cache:
        salvato = caselle.leggi_cache_eventi().get(chiave_cache)
        if salvato:
            meta["stale"] = True
            meta["stale_at"] = datetime.fromtimestamp(
                salvato.get("stamp", 0)).isoformat(timespec="seconds")
            return salvato.get("eventi", []), [str(errore)], meta
    return [], [str(errore)], meta


def fetch_all_events(cfg, start_iso, end_iso, allow_cache=True):
    """Eventi di tutte le caselle lette dall'agente, nell'intervallo dato.

    Ritorna (eventi, errori, meta). In più, meta["stato_aziende"] dà il pallino di
    ogni azienda per QUESTA lettura.
    """
    meta = {"stale": False, "stale_at": None, "outlook_starting": False}
    try:
        start = caselle.parse_bound(start_iso)
        end = caselle.parse_bound(end_iso)
    except (ValueError, AttributeError) as e:
        return [], [f"Intervallo non valido: {e}"], meta
    chiave_cache = f"{start.isoformat()}|{end.isoformat()}"
    try:
        dati = _chiedi("/api/eventi", {"inizio": start.isoformat(), "fine": end.isoformat()})
    except AgenteNonDisponibile as e:
        return _senza_agente(cfg, e, chiave_cache, allow_cache, meta)

    disabled = set(cfg.get("disabled_calendars", []))
    servizio = dati.get("servizio")
    visibili = _caselle_visibili(dati)
    _ricorda(visibili)
    mie = {c["account"].lower() for c in visibili}
    alias_casella = config.alias_per_account(cfg, sorted(mie))
    # Per ogni azienda: se ha almeno una casella accesa, e com'e' andata la
    # lettura. Anche le caselle spente si leggono: possono contenere riunioni di
    # un'ALTRA azienda, che devono restare visibili se quella e' accesa (a
    # scartare pensa il filtro per azienda, a valle della riclassificazione).
    global _ultimo_stato
    aziende, stato_aziende, errors, stati = {}, {}, [], {}
    for c in visibili:
        account = c["account"].lower()
        azienda = alias_casella[account]
        stato = _stato_casella(c, servizio)
        stati[account] = stato
        aziende[azienda] = aziende.get(azienda, False) or _id_calendario(account) not in disabled
        stato_aziende[azienda] = _peggiore(stato_aziende.get(azienda), stato)
        if stato["stato"] == "errore":
            errors.append(f"{azienda}: {stato['msg']}")
    # Sostituzione in un colpo solo: l'interfaccia e il servizio promemoria
    # leggono da thread diversi e non devono mai trovare l'elenco a meta'.
    _ultimo_stato = stati

    events, seen = [], {}
    for e in dati.get("eventi") or []:
        account = str(e.get("account") or "").lower()
        if account not in mie or not e.get("start"):
            continue
        # Il nome del calendario lo dice l'agente: una casella ne può portare più
        # d'uno (i calendari condivisi da altre aziende), e azienda_per_cartella
        # li riconosce da lì.
        label = e.get("calendar") or f"{account} · Calendario"
        ev = {
            "id": e.get("id") or f"{account}|{e.get('start')}",
            "account": account,
            "calendar": label,
            "color": config.color_for_account(cfg, account),
            "title": e.get("title") or "(senza titolo)",
            "start": e["start"],
            "end": e.get("end") or e["start"],
            "allDay": bool(e.get("allDay")),
            "location": e.get("location") or "",
            "organizer": e.get("organizer") or "",
            "organizer_email": e.get("organizer_email") or "",
            "online_url": e.get("online_url"),
            "web_link": None,
            # Non c'e' un EntryID di Outlook: l'interfaccia lo deve sapere.
            "remoto": True,
        }
        if e.get("tipo") == "caldav":
            # Calendario Google: è il calendario dell'azienda e la sua etichetta
            # vince su quella della casella in cui Outlook ha archiviato lo stesso
            # invito.
            ev["is_google"] = True
        invitati = [str(x).lower() for x in (e.get("attendees") or [])]
        caselle.accoda(events, seen, ev, label,
                       lambda invitati=invitati: next((i for i in invitati if i in mie), ""))

    events.sort(key=lambda e: e["start"] or "")
    caselle.etichetta(cfg, events)
    meta["aziende"] = aziende
    meta["stato_aziende"] = stato_aziende
    if allow_cache:
        caselle.scrivi_cache_eventi(chiave_cache, events)
    return events, errors, meta
