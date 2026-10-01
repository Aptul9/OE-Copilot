"""Lettura diretta dei calendari Google via CalDAV.

Perche' esiste: un account Google in Outlook e' spesso una casella IMAP, che non
puo' contenere calendari. Le alternative erano replicare il calendario dentro la
casella Exchange di un'altra azienda (dati mescolati) o agganciare un archivio
.pst dedicato, che pero' resta visibile nell'elenco cartelle di Outlook. Qui si
salta del tutto Outlook: si parla direttamente con Google.

Autenticazione: si riusa il token gia' rilasciato a CalDav Synchronizer, che la
libreria Google salva in chiaro in %APPDATA%\\Google.Apis.Auth\\. Nessuna
credenziale viene chiesta all'utente e nessuna viene salvata qui: si legge quel
file, si rinnova l'access token quando scade e basta. Se il file non c'e', il
calendario semplicemente non compare (nessun errore bloccante).

Nota: si usa CalDAV e non l'API REST di Google Calendar perche' il progetto
OAuth di CalDav Synchronizer non ha quell'API abilitata (risponde 403), mentre
l'endpoint CalDAV e' esattamente quello per cui il token e' stato emesso.
"""
import json
import logging
import os
import re
import threading
import time
from datetime import date, datetime, timedelta, timezone
from xml.etree import ElementTree

import icalendar
import recurring_ical_events
import requests

import config
from ics_feed import MEETING_URL_RE, _fuso_locale, _naive_locale, _testo

log = logging.getLogger(__name__)

# Client OAuth di CalDav Synchronizer (open source: sono valori incorporati nel
# programma installato, non segreti dell'utente). Servono solo a rinnovare il
# token gia' rilasciato a quell'applicazione.
CLIENT_ID = "13856942399-rp437ddbn6406hfokpe5rqnosgnejodc.apps.googleusercontent.com"
CLIENT_SECRET = "WG276vw5WCcc2H4SSaYJ03VO"
TOKEN_URL = "https://oauth2.googleapis.com/token"
CALDAV_BASE = "https://apidata.googleusercontent.com/caldav/v2"

_ns = {"d": "DAV:", "c": "urn:ietf:params:xml:ns:caldav"}

_lock = threading.Lock()
_token_cache = {}  # indirizzo -> (access_token, scadenza epoch)

# Stato dell'integrazione, per farlo vedere nell'app: un token che smette di
# funzionare non deve tradursi in eventi che spariscono in silenzio.
# indirizzo -> {"stato", "messaggio", "ultimo_ok", "scade"}
#   "ok"        tutto a posto
#   "riautorizzare"  il token e' stato revocato/scaduto: serve l'intervento
#   "problema"  guasto temporaneo (rete, server): si riprova da solo
#   "assente"   nessun token per quell'indirizzo
_stato = {}


def _segna(indirizzo, stato, messaggio, scade=None, ok=False):
    voce = _stato.setdefault(indirizzo, {"ultimo_ok": None})
    voce.update({"stato": stato, "messaggio": messaggio, "scade": scade})
    if ok:
        voce["ultimo_ok"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    return voce


def _protetto(testo):
    """Cifra con DPAPI: il risultato lo apre solo questo utente su questo PC."""
    import ctypes
    from ctypes import wintypes

    class BLOB(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD),
                    ("pbData", ctypes.POINTER(ctypes.c_char))]

    grezzo = testo.encode("utf-8")
    dentro = BLOB(len(grezzo), ctypes.cast(ctypes.create_string_buffer(grezzo),
                                           ctypes.POINTER(ctypes.c_char)))
    fuori = BLOB()
    if not ctypes.windll.crypt32.CryptProtectData(
            ctypes.byref(dentro), "OE Copilot", None, None, None, 0,
            ctypes.byref(fuori)):
        raise OSError("CryptProtectData non riuscita")
    try:
        return bytes(ctypes.string_at(fuori.pbData, fuori.cbData))
    finally:
        ctypes.windll.kernel32.LocalFree(fuori.pbData)


def _svelato(dati):
    """Decifra quanto scritto da _protetto. Solleva se non e' il nostro utente."""
    import ctypes
    from ctypes import wintypes

    class BLOB(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD),
                    ("pbData", ctypes.POINTER(ctypes.c_char))]

    dentro = BLOB(len(dati), ctypes.cast(ctypes.create_string_buffer(dati),
                                         ctypes.POINTER(ctypes.c_char)))
    fuori = BLOB()
    if not ctypes.windll.crypt32.CryptUnprotectData(
            ctypes.byref(dentro), None, None, None, None, 0,
            ctypes.byref(fuori)):
        raise OSError("CryptUnprotectData non riuscita")
    try:
        return bytes(ctypes.string_at(fuori.pbData, fuori.cbData)).decode("utf-8")
    finally:
        ctypes.windll.kernel32.LocalFree(fuori.pbData)


def _percorso_riserva(indirizzo):
    """Copia di riserva del token, nella cartella dati dell'app.

    Il file originale (scritto da CalDav Synchronizer in %APPDATA%) a tratti non
    e' raggiungibile: la stessa lettura fallisce con "file non trovato" e un
    minuto dopo riesce. Il contenuto pero' e' una stringa che non cambia mai
    (refresh_token), quindi se ne tiene una copia qui e la si usa quando
    l'originale non risponde. Sta sullo stesso PC e nello stesso profilo utente
    dell'originale, quindi non cambia chi puo' leggerla.
    """
    sicuro = re.sub(r"[^A-Za-z0-9._@-]", "_", indirizzo)
    return os.path.join(config.APP_DIR, f"google_token_{sicuro}.bin")


def _percorso_token(indirizzo):
    return os.path.join(
        os.environ.get("APPDATA", ""), "Google.Apis.Auth",
        f"Google.Apis.Auth.OAuth2.Responses.TokenResponse-{indirizzo}")


def token_disponibile(indirizzo):
    return not _motivo_token_mancante(indirizzo)


def _motivo_token_mancante(indirizzo):
    """"" se il token si legge, altrimenti il motivo preciso.

    Non basta os.path.exists: se un antivirus o un criterio di Windows nega
    l'accesso alla cartella, exists() torna False e sembra che il file non ci
    sia. Sono due guasti diversi e vanno detti in modo diverso, perche' si
    risolvono in modo diverso.
    """
    percorso = _percorso_token(indirizzo)
    try:
        with open(percorso, "r", encoding="utf-8"):
            return ""
    except FileNotFoundError:
        return "autorizzazione Google non trovata"
    except PermissionError as e:
        return f"file dell'autorizzazione non leggibile (accesso negato: {e.strerror})"
    except OSError as e:
        return f"file dell'autorizzazione non leggibile ({e.strerror})"


_diagnosi_fatta = False


def _diagnostica_token(percorso):
    """Fotografa cosa vede QUESTO processo quando il file non si apre.

    Serve perche' lo stesso file, nello stesso istante, si legge da un altro
    processo: la differenza va cercata nell'ambiente del processo, non nel file.
    """
    global _diagnosi_fatta
    if _diagnosi_fatta:
        return      # una volta per avvio basta: ora c'e' la copia di riserva
    _diagnosi_fatta = True
    import ctypes
    cartella = os.path.dirname(percorso)
    try:
        elenco = os.listdir(cartella)
    except OSError as e:
        elenco = f"(cartella non elencabile: {e})"
    try:
        # Apertura Win32 diretta: il codice d'errore dice piu' di errno.
        h = ctypes.windll.kernel32.CreateFileW(percorso, 0x80000000, 0x7, None,
                                               3, 0x80, None)
        errore_win = ctypes.get_last_error() if h == -1 else 0
        if h != -1:
            ctypes.windll.kernel32.CloseHandle(h)
        apertura = "riuscita" if h != -1 else f"fallita (errore Windows {errore_win})"
    except Exception as e:
        apertura = f"non tentata ({e})"
    log.error("DIAGNOSI TOKEN - questo processo NON apre un file che esiste:")
    log.error("   APPDATA nel processo : %s", os.environ.get("APPDATA"))
    log.error("   percorso cercato     : %s", percorso)
    log.error("   cartella esiste      : %s", os.path.isdir(cartella))
    log.error("   contenuto cartella   : %s", elenco)
    log.error("   exists() sul file    : %s", os.path.exists(percorso))
    log.error("   apertura Win32       : %s", apertura)
    log.error("   utente               : %s", os.environ.get("USERNAME"))


def _leggi_token(indirizzo, tentativi=3):
    """Contenuto del token: prima l'originale, poi la copia di riserva.

    Si ritenta perche' capita che la lettura fallisca con "file non trovato" su
    un file che esiste eccome. Se anche i tentativi falliscono si usa la copia
    che l'app tiene per conto suo: e' l'unico modo per non dipendere da un file
    che a tratti sparisce.
    """
    dati, motivo = _leggi_token_originale(indirizzo, tentativi)
    if dati is not None:
        _aggiorna_riserva(indirizzo, dati)
        return dati, ""
    riserva = _percorso_riserva(indirizzo)
    try:
        with open(riserva, "rb") as f:
            dati = json.loads(_svelato(f.read()))
        log.warning("google_cal: originale non leggibile (%s), uso la copia di "
                    "riserva", motivo)
        return dati, ""
    except FileNotFoundError:
        return None, motivo
    except Exception as e:
        log.warning("google_cal: copia di riserva inutilizzabile (%s)", e)
        return None, motivo


def _aggiorna_riserva(indirizzo, dati):
    """Tiene aggiornata la copia, ma solo se il contenuto e' cambiato."""
    riserva = _percorso_riserva(indirizzo)
    try:
        with open(riserva, "rb") as f:
            if json.loads(_svelato(f.read())) == dati:
                return
    except Exception:
        pass
    try:
        with open(riserva, "wb") as f:
            f.write(_protetto(json.dumps(dati)))
        log.info("google_cal: copia di riserva del token aggiornata")
    except Exception:
        log.warning("google_cal: copia di riserva non scritta", exc_info=True)


def _leggi_token_originale(indirizzo, tentativi=3):
    percorso = _percorso_token(indirizzo)
    ultimo = ""
    for n in range(tentativi):
        try:
            with open(percorso, "r", encoding="utf-8") as f:
                dati = json.load(f)
            if n:
                log.info("google_cal: token letto al tentativo %d", n + 1)
            return dati, ""
        except FileNotFoundError as e:
            ultimo = "Autorizzazione Google non trovata"
            log.warning("google_cal: token non trovato (tentativo %d/%d): %s",
                        n + 1, tentativi, e)
            if n == 0:
                _diagnostica_token(percorso)
        except PermissionError as e:
            ultimo = f"File dell'autorizzazione non leggibile ({e.strerror})"
            log.warning("google_cal: token non leggibile (tentativo %d/%d): %s",
                        n + 1, tentativi, e)
        except (OSError, json.JSONDecodeError) as e:
            ultimo = f"Autorizzazione Google illeggibile ({e})"
            log.warning("google_cal: token rotto: %s", e)
            break
        time.sleep(0.3)
    return None, ultimo


def _access_token(indirizzo):
    """Access token valido, rinnovandolo se serve. None se non c'e' il refresh."""
    with _lock:
        cached = _token_cache.get(indirizzo)
        if cached and cached[1] > time.time() + 60:
            return cached[0]
        dati, motivo = _leggi_token(indirizzo)
        if dati is None:
            _segna(indirizzo, "assente", motivo)
            return None
        refresh = dati.get("refresh_token")
        if not refresh:
            _segna(indirizzo, "riautorizzare", "Autorizzazione incompleta")
            return None
        try:
            r = requests.post(TOKEN_URL, timeout=30, data={
                "client_id": CLIENT_ID, "client_secret": CLIENT_SECRET,
                "refresh_token": refresh, "grant_type": "refresh_token"})
        except requests.RequestException as e:
            log.warning("google_cal: rinnovo token fallito (%s)", e)
            _segna(indirizzo, "problema", f"Google non raggiungibile ({e.__class__.__name__})")
            return None
        if r.status_code != 200:
            # Va distinto il token morto (serve l'utente) dal guasto passeggero.
            motivo = ""
            try:
                motivo = (r.json() or {}).get("error", "")
            except ValueError:
                pass
            log.warning("google_cal: rinnovo rifiutato (HTTP %s, %s)", r.status_code, motivo)
            if motivo in ("invalid_grant", "unauthorized_client", "invalid_client"):
                _segna(indirizzo, "riautorizzare",
                       "Autorizzazione Google scaduta o revocata")
            else:
                _segna(indirizzo, "problema", f"Rinnovo rifiutato (HTTP {r.status_code})")
            return None
        risposta = r.json()
        at = risposta.get("access_token")
        durata = int(risposta.get("expires_in") or 3600)
        scadenza = time.time() + durata
        _token_cache[indirizzo] = (at, scadenza)
        _segna(indirizzo, "ok", "Collegato",
               scade=time.strftime("%H:%M", time.localtime(scadenza)))
        return at


def _query_caldav(indirizzo, token, start, end, tentativi=2):
    """Oggetti iCalendar del periodo. Ritorna lista di testi .ics.

    Ritenta una volta: un intoppo di rete o un 5xx passeggero non devono far
    sparire il calendario per un minuto intero.
    """
    ultimo = None
    for n in range(tentativi):
        try:
            return _query_caldav_una_volta(indirizzo, token, start, end)
        except (requests.RequestException, RuntimeError) as e:
            ultimo = e
            log.warning("google_cal: lettura CalDAV fallita (tentativo %d/%d): %s",
                        n + 1, tentativi, e)
            time.sleep(1)
    raise ultimo


def _query_caldav_una_volta(indirizzo, token, start, end):
    url = f"{CALDAV_BASE}/{indirizzo}/events/"
    corpo = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">'
        '<d:prop><d:getetag/><c:calendar-data/></d:prop>'
        '<c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">'
        f'<c:time-range start="{start.strftime("%Y%m%dT%H%M%SZ")}"'
        f' end="{end.strftime("%Y%m%dT%H%M%SZ")}"/>'
        '</c:comp-filter></c:comp-filter></c:filter></c:calendar-query>')
    r = requests.request(
        "REPORT", url, timeout=60, data=corpo.encode("utf-8"),
        headers={"Authorization": f"Bearer {token}", "Depth": "1",
                 "Content-Type": 'application/xml; charset=utf-8'})
    if r.status_code >= 400:
        raise RuntimeError(f"CalDAV HTTP {r.status_code}")
    radice = ElementTree.fromstring(r.content)
    return [n.text for n in radice.iter(f"{{{_ns['c']}}}calendar-data") if n.text]


def _pallino(indirizzo):
    """Stato da mostrare accanto al calendario, basato sull'ULTIMA lettura vera.

    Prima era verde per default, anche prima di aver letto qualcosa: un
    calendario che non veniva mai interrogato sembrava funzionante mentre le sue
    riunioni non comparivano. Verde solo con una lettura riuscita alle spalle.
    """
    voce = _stato.get(indirizzo) or {}
    stato = voce.get("stato")
    if stato == "ok":
        quando = voce.get("ultimo_ok")
        return {"stato": "ok",
                "stato_msg": ("Google letto alle " + quando[11:16]) if quando
                             else "Google collegato"}
    if stato is None:
        return {"stato": "avviso", "stato_msg": "Non ancora letto"}
    if stato == "problema":
        # Guasto passeggero (rete assente, Google non raggiungibile): giallo, non
        # rosso. Rosso vuol dire "serve che tu faccia qualcosa".
        return {"stato": "avviso", "stato_msg": voce.get("messaggio") or "Problema temporaneo"}
    return {"stato": "errore", "stato_msg": voce.get("messaggio") or "Non raggiungibile"}


def calendari(cfg):
    """Voci per l'elenco calendari, una per account Google configurato."""
    disabled = set(cfg.get("disabled_calendars", []))
    out = []
    for c in config.CALENDARI_GOOGLE:
        # Il calendario resta in elenco anche se in questo istante il token non
        # si legge: prima spariva dalla barra laterale (e con lui il suo pallino)
        # proprio quando c'era un problema da mostrare.
        out.append({
            "id": c["id"],
            "account": c["nome"],
            "label": c["nome"],
            "count": None,           # via CalDAV non c'e' un conteggio immediato
            "is_google": True,
            "color": config.color_for_account(cfg, c["nome"]),
            "enabled": c["id"] not in disabled,
            "alias": c["nome"],
            **_pallino(c["indirizzo"]),
        })
    return out


def stato_integrazioni(verifica=False):
    """Stato di ogni calendario Google, per mostrarlo nell'app.

    Con verifica=True prova davvero a rinnovare il token invece di riferire
    l'esito dell'ultima lettura: serve al pulsante "Verifica adesso".
    """
    out = []
    for c in config.CALENDARI_GOOGLE:
        ind = c["indirizzo"]
        if verifica:
            _token_cache.pop(ind, None)   # forza un rinnovo vero
            _access_token(ind)
        voce = dict(_stato.get(ind) or {})
        if not voce:
            # Nessuna lettura ancora fatta: si dice quello, non si inventa un
            # verde. Il file lo si guarda solo qui, dove serve davvero.
            motivo = _motivo_token_mancante(ind)
            voce = {"stato": "avviso" if not motivo else "assente",
                    "messaggio": "Non ancora letto" if not motivo else motivo.capitalize(),
                    "ultimo_ok": None, "scade": None}
        voce.update({"nome": c["nome"], "indirizzo": ind, "tipo": "Google"})
        out.append(voce)
    return out


def fetch_events(cfg, start, end):
    """Eventi dei calendari Google attivi. Ritorna (eventi, errori).

    start/end sono datetime locali naive, come per la lettura da Outlook.
    """
    disabled = set(cfg.get("disabled_calendars", []))
    fuso = _fuso_locale()
    eventi, errori = [], []

    for c in config.CALENDARI_GOOGLE:
        # Saltare in silenzio era un buco: se il calendario risultava spento o il
        # file del token spariva, gli eventi non comparivano piu' e nel log non
        # restava traccia di nulla. Ora il motivo si vede sempre.
        if c["id"] in disabled:
            log.info("google_cal: %s saltato, calendario spento", c["nome"])
            continue
        nome = c["nome"]
        # NIENTE controllo del file a ogni giro: finche' l'access token in
        # memoria e' valido (circa un'ora) il file non serve. Prima si rileggeva
        # ogni minuto e un singolo intoppo nel leggerlo buttava giu' il calendario anche
        # quando c'era tutto il necessario per funzionare.
        token = _access_token(c["indirizzo"])
        if not token:
            voce = _stato.get(c["indirizzo"], {})
            errori.append(f"{nome}: {voce.get('messaggio', 'autorizzazione da rifare')}")
            continue
        # La query CalDAV vuole istanti UTC (formato "...Z"). Qui start/end sono
        # ora locale: vanno convertiti davvero in UTC. Prima si usava
        # `astimezone(tz=None)`, che per un orario gia' locale non cambia nulla e
        # lasciava spedire l'ora locale spacciata per UTC: la finestra risultava
        # spostata di un fuso e le riunioni delle prime ore del primo giorno non
        # tornavano indietro. Si allarga anche di un giorno per lato, cosi' gli
        # eventi a cavallo del bordo ci sono comunque (il filtro fine lo fa poi
        # recurring_ical_events sull'intervallo esatto).
        margine = timedelta(days=1)
        try:
            testi = _query_caldav(
                c["indirizzo"], token,
                (start.replace(tzinfo=fuso) - margine).astimezone(timezone.utc).replace(tzinfo=None),
                (end.replace(tzinfo=fuso) + margine).astimezone(timezone.utc).replace(tzinfo=None))
        except Exception as e:
            _segna(c["indirizzo"], "problema", f"Lettura calendario fallita ({e})")
            errori.append(f"{nome}: {e}")
            continue
        _segna(c["indirizzo"], "ok", "Collegato",
               scade=_stato.get(c["indirizzo"], {}).get("scade"), ok=True)

        colore = config.color_for_account(cfg, nome)
        scartati = 0
        for testo in testi:
            try:
                cal = icalendar.Calendar.from_ical(testo)
                trovati = recurring_ical_events.of(cal).between(
                    start.replace(tzinfo=fuso), end.replace(tzinfo=fuso))
            except Exception:
                # Un singolo oggetto malformato non ferma gli altri, ma va
                # contato: se sono tanti, il calendario sembra mezzo vuoto e
                # senza questo conteggio non si capirebbe perche'.
                scartati += 1
                continue
            for ev in trovati:
                dtstart = ev.get("DTSTART")
                inizio = _naive_locale(dtstart.dt if dtstart else None)
                if inizio is None:
                    continue
                dtend = ev.get("DTEND")
                fine_ev = _naive_locale(dtend.dt if dtend else None) or inizio
                tutto_giorno = isinstance(dtstart.dt, date) and not isinstance(
                    dtstart.dt, datetime)
                descrizione = _testo(ev, "DESCRIPTION")
                luogo = _testo(ev, "LOCATION")
                link = MEETING_URL_RE.search(luogo) or MEETING_URL_RE.search(descrizione)
                eventi.append({
                    "id": f"{c['id']}|{_testo(ev, 'UID')}|{inizio.isoformat()}",
                    "account": nome,
                    "alias": nome,
                    "calendar": nome,
                    "color": colore,
                    "title": _testo(ev, "SUMMARY") or "(senza titolo)",
                    "start": inizio.isoformat(),
                    "end": fine_ev.isoformat(),
                    "allDay": bool(tutto_giorno),
                    "location": luogo,
                    "organizer": _testo(ev, "ORGANIZER").replace("mailto:", ""),
                    "organizer_email": _testo(ev, "ORGANIZER").replace("mailto:", ""),
                    "online_url": link.group(0).rstrip(">).,;\"'") if link else None,
                    "web_link": None,
                    "is_google": True,   # per la deduplica: l'azienda vince sulla casella
                })
        if scartati:
            log.warning("google_cal: %s, %d oggetti non leggibili su %d",
                        nome, scartati, len(testi))
    return eventi, errori
