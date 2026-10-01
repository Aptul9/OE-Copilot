r"""Gestione configurazione e percorsi dell'app.

Percorsi: due radici distinte, perché da eseguibile (PyInstaller) l'app gira da
una cartella di sola lettura.
  - RESOURCE_DIR: file inclusi nel pacchetto (web/, icona). Da eseguibile è la
    cartella temporanea di estrazione (sys._MEIPASS); da sorgente è questa.
  - DATA_DIR: file scrivibili (config, cache, log, eventi locali). Da eseguibile
    è %LOCALAPPDATA%\CalendarHub, così sopravvive agli aggiornamenti dell'exe e
    funziona anche se l'app è installata in una cartella protetta. Da sorgente
    resta la cartella del progetto, come prima.
"""
import json
import logging
import os
import re
import shutil
import sys
import tempfile
import threading
import time

log = logging.getLogger(__name__)

_QUI = os.path.dirname(os.path.abspath(__file__))

if getattr(sys, "frozen", False):
    RESOURCE_DIR = getattr(sys, "_MEIPASS", _QUI)
    DATA_DIR = os.path.join(
        os.environ.get("LOCALAPPDATA", os.path.expanduser("~")), "CalendarHub")
else:
    RESOURCE_DIR = _QUI
    DATA_DIR = _QUI

os.makedirs(DATA_DIR, exist_ok=True)

# Compatibilità: il resto del codice usava APP_DIR per i file scrivibili.
APP_DIR = DATA_DIR

CONFIG_PATH = os.path.join(DATA_DIR, "config.json")
FIRED_REMINDERS_PATH = os.path.join(DATA_DIR, "fired_reminders.json")

# Palette usata per assegnare automaticamente un colore a ogni account.
# Deve avere piu' tinte del numero di account, altrimenti due caselle diverse
# finiscono dello stesso colore e la legenda diventa ambigua.
COLOR_PALETTE = [
    "#2563eb",  # blu
    "#16a34a",  # verde
    "#db2777",  # rosa
    "#ea580c",  # arancio
    "#7c3aed",  # viola
    "#0891b2",  # ciano
    "#ca8a04",  # ocra
    "#dc2626",  # rosso
    "#4d7c0f",  # verde oliva
    "#9333ea",  # magenta
    "#0f766e",  # verde acqua
    "#b45309",  # bronzo
    "#1e40af",  # blu notte
    "#be123c",  # rubino
    "#525252",  # grigio
    "#65a30d",  # lime
    "#0d9488",  # teal
    "#c026d3",  # fucsia
    "#f59e0b",  # ambra
    "#6366f1",  # indaco
    "#e11d48",  # rosa acceso
    "#15803d",  # verde bosco
    "#a16207",  # senape
    "#7e22ce",  # viola scuro
    "#0369a1",  # blu petrolio
]

DEFAULT_CONFIG = {
    # Nessuna credenziale: gli eventi arrivano da Outlook desktop via COM, quindi
    # valgono gli account già configurati nel profilo Outlook. Il calendario
    # Google entra come calendario Internet (ICS) sottoscritto dentro Outlook.
    # Calendari da NON mostrare: elenco di EntryID (si gestisce dalla UI).
    "disabled_calendars": [],
    # Profilo Outlook da usare. Vuoto = quello predefinito. Va valorizzato solo
    # se sul PC ci sono più profili e si vuole imporre quale leggere.
    "outlook_profile": "",
    # Calendari sottoscritti da link iCal: [{"id", "name", "url"}].
    # Servono per Google e in generale per gli account che Outlook non porta.
    "ics_calendars": [],
    # Come chiamare gli account: {"mario@azienda.it": "Azienda"}. Vuoto = nome
    # dedotto dal dominio. Si modifica dalla finestra "Calendari".
    "account_aliases": {},
    # Quanti minuti prima dell'evento inviare il promemoria (puoi metterne più di uno).
    "reminder_minutes_before": [10],
    # Ogni quanti secondi il servizio controlla gli eventi imminenti.
    "reminder_poll_seconds": 60,
    "windows_toast": True,
    "telegram": {"enabled": False, "bot_token": "", "chat_id": ""},
    # Colore per account: {"mario@x.com": "#2563eb"}. Riempito automaticamente.
    "account_colors": {},
    # Aspetto: "auto" (segue Windows), "light" o "dark".
    "theme": "auto",
    # All'avvio parti ridotto a icona (utile con l'avvio automatico).
    "start_minimized": False,
    # Zoom del calendario: altezza in pixel della riga da mezz'ora (pulsanti +/-).
    "zoom_calendario": 58,
}


# La configurazione la scrivono piu' thread (interfaccia e servizio promemoria,
# per esempio quando a un account nuovo viene assegnato un colore): senza lock
# due salvataggi ravvicinati possono sovrapporsi e perdere delle voci.
_lock_config = threading.Lock()


def scrivi_json(percorso, dati):
    """Salvataggio atomico: prima un file temporaneo, poi la sostituzione.

    Scrivendo direttamente sul file finale, un'interruzione a meta' (o due
    scritture insieme) lascia un JSON troncato: al riavvio non si rilegge piu' e
    le impostazioni sembrano azzerate. os.replace e' atomico, quindi il file o e'
    quello vecchio o e' quello nuovo, mai a meta'.
    """
    cartella = os.path.dirname(percorso) or "."
    fd, tmp = tempfile.mkstemp(dir=cartella, prefix=".tmp_", suffix=".json")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(dati, f, indent=2, ensure_ascii=False)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, percorso)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def load_config():
    cfg = dict(DEFAULT_CONFIG)
    if os.path.exists(CONFIG_PATH):
        try:
            with open(CONFIG_PATH, "r", encoding="utf-8") as f:
                cfg.update(json.load(f))
        except json.JSONDecodeError:
            # File illeggibile: se ne tiene una copia PRIMA di ripartire dai
            # valori di fabbrica, altrimenti il primo salvataggio successivo lo
            # sovrascriverebbe e le impostazioni sarebbero perse davvero.
            guasto = f"{CONFIG_PATH}.rotto-{time.strftime('%Y%m%d-%H%M%S')}"
            log.error("configurazione illeggibile: ne salvo una copia in %s "
                      "e riparto dai valori predefiniti", guasto)
            try:
                shutil.copy2(CONFIG_PATH, guasto)
            except OSError:
                log.warning("copia della configurazione rotta non riuscita",
                            exc_info=True)
        except OSError:
            log.warning("configurazione non leggibile", exc_info=True)
    # Garantisce che i sotto-dizionari esistano sempre.
    cfg.setdefault("telegram", dict(DEFAULT_CONFIG["telegram"]))
    cfg.setdefault("account_colors", {})
    cfg.setdefault("disabled_calendars", [])
    cfg.setdefault("ics_calendars", [])
    cfg.setdefault("account_aliases", {})
    return cfg


# ---------------------------------------------------------------------------
# Personalizzazione: aziende, caselle e domini di chi usa l'app
# ---------------------------------------------------------------------------
# Non stanno nel codice ma in personalizzazione.json, accanto ai dati dell'app
# (escluso dal repository: sono dati di una persona). Senza quel file l'app
# funziona lo stesso e i calendari prendono il nome dal dominio della casella.
# Il formato, con un esempio per ogni voce, e' in personalizzazione.esempio.json.
# OE_PERSONALIZZAZIONE permette di indicare un altro file (serve alle prove).
PERSONALIZZAZIONE_PATH = os.environ.get(
    "OE_PERSONALIZZAZIONE", os.path.join(DATA_DIR, "personalizzazione.json"))


def _leggi_personalizzazione():
    try:
        with open(PERSONALIZZAZIONE_PATH, "r", encoding="utf-8") as f:
            dati = json.load(f)
        if isinstance(dati, dict):
            return dati
        log.error("personalizzazione: atteso un oggetto JSON, file ignorato")
    except FileNotFoundError:
        pass
    except (OSError, json.JSONDecodeError):
        # Un file rotto non deve fermare l'app: si parte senza personalizzazione,
        # ma lo si scrive nel log, perche' i nomi delle aziende spariranno.
        log.error("personalizzazione non leggibile, uso i valori predefiniti",
                  exc_info=True)
    return {}


_P = _leggi_personalizzazione()


def _minuscole(voci):
    return {str(v).strip().lower() for v in (voci or []) if str(v).strip()}


# Nome da mostrare per una casella. La chiave e' l'indirizzo intero quando piu'
# caselle dello stesso dominio vanno distinte (per esempio la propria casella e
# due caselle condivise della stessa azienda), altrimenti basta il dominio.
NOMI_PER_INDIRIZZO = {str(k).strip().lower(): v for k, v in
                      (_P.get("nomi_per_indirizzo") or {}).items()}
NOMI_PER_DOMINIO = {str(k).strip().lower(): v for k, v in
                    (_P.get("nomi_per_dominio") or {}).items()}

# Caselle di COLLEGHI presenti nel profilo Outlook. Il nome in NOMI_PER_INDIRIZZO
# serve solo a distinguere il loro calendario nell'elenco; per il resto contano
# come la loro azienda (una riunione organizzata da loro e' dell'azienda) e gli
# eventi del loro calendario restano loro, senza essere riattribuiti.
CASELLE_DI_COLLEGHI = _minuscole(_P.get("caselle_di_colleghi"))

MESI_IT = ["Gennaio", "Febbraio", "Marzo", "Aprile", "Maggio", "Giugno",
           "Luglio", "Agosto", "Settembre", "Ottobre", "Novembre", "Dicembre"]

# Ordine con cui mostrare i calendari in "I miei calendari" (per nome mostrato).
# Chi non è in elenco finisce in fondo, nell'ordine di Outlook.
ORDINE_CALENDARI = list(_P.get("ordine_calendari") or ["Eventi personali", "Festività"])


def ordine_calendario(alias):
    try:
        return ORDINE_CALENDARI.index(alias)
    except ValueError:
        return len(ORDINE_CALENDARI)


# Domini che identificano una "azienda" vera (non provider di posta generici).
# Servono per capire di chi è una riunione guardando CHI la organizza, non la
# casella in cui è finita: un invito accettato da Outlook può essere archiviato
# nel calendario predefinito, che è di un'altra azienda, ma il dominio
# dell'organizzatore lo smaschera.
DOMINI_AZIENDALI = _minuscole(_P.get("domini_aziendali"))


# Store "Cartelle pubbliche"/"Public Folders": non sono la casella dell'utente,
# vanno saltati (altrimenti spuntano calendari pubblici non suoi nell'elenco).
CARTELLE_PUBBLICHE = ("cartelle pubbliche", "public folders")


def store_pubblico(nome):
    n = (nome or "").lower()
    return any(k in n for k in CARTELLE_PUBBLICHE)


# Cartelle calendario dedicate a un'azienda: gli eventi che ci stanno dentro sono
# di quell'azienda, a prescindere da store che le ospita e da chi risulta
# organizzatore. Serve per i calendari sincronizzati da programmi esterni (per
# esempio CalDav Synchronizer), che vengono creati come sottocartelle di un'altra
# casella e perdono l'organizzatore reale. {"testo nel nome cartella": "Azienda"}
AZIENDA_PER_CARTELLA = dict(_P.get("azienda_per_cartella") or {})


def azienda_da_cartella(label_o_nome):
    """Azienda dedotta dal nome della cartella calendario, o None."""
    if not label_o_nome:
        return None
    testo = label_o_nome.lower()
    for nome, azienda in AZIENDA_PER_CARTELLA.items():
        if nome.lower() in testo:
            return azienda
    return None


# Aziende SENZA una casella in Outlook, i cui impegni arrivano copiati in
# un'altra casella da un flusso (per esempio Power Automate) e si riconoscono dal
# prefisso nel titolo. Tipico quando un account e' bloccato alle app e si apre
# solo dal browser: un flusso copia ogni riunione nuova nel calendario di
# un'altra casella con un prefisso nel titolo, e un flusso gemello inoltra la
# posta con un prefisso nell'oggetto. Le copie si mostrano come l'azienda
# d'origine, col titolo vero; la posta inoltrata dice se il collegamento e' vivo
# (outlook_com). Voci in personalizzazione.json:
#   "via":           casella in cui arrivano le copie
#   "prefisso":      espressione regolare del prefisso nel titolo delle copie
#   "oggetto_posta": inizio dell'oggetto delle mail inoltrate (facoltativo)
#   "colore":        colore da dare all'azienda alla prima assegnazione
def _aziende_inoltrate():
    out = {}
    for nome, d in (_P.get("aziende_inoltrate") or {}).items():
        if str(nome).startswith("_"):
            continue
        try:
            out[nome] = {"via": d["via"],
                         "prefisso": re.compile(d["prefisso"], re.I),
                         "oggetto_posta": d.get("oggetto_posta") or "",
                         "colore": d.get("colore")}
        except (KeyError, TypeError, AttributeError, re.error):
            log.error("personalizzazione: azienda inoltrata %r non valida, ignorata",
                      nome, exc_info=True)
    return out


AZIENDE_INOLTRATE = _aziende_inoltrate()


def azienda_da_titolo(titolo):
    """(azienda, titolo senza prefisso) per le copie dei flussi, se no (None, titolo)."""
    for azienda, d in AZIENDE_INOLTRATE.items():
        m = d["prefisso"].match(titolo or "")
        if m:
            return azienda, (titolo[m.end():].strip() or "(senza titolo)")
    return None, titolo


def colore_per_azienda(cfg, azienda):
    """Colore già assegnato a una tua casella di quell'azienda, o None."""
    colori = cfg.get("account_colors", {})
    alias = alias_per_account(cfg, list(colori.keys()))
    for acc, col in colori.items():
        if alias.get(acc) == azienda:
            return col
    return None


def azienda_da_email(email):
    """Nome azienda dedotto dall'indirizzo dell'organizzatore, o None.

    Ritorna un nome solo per i domini aziendali (DOMINI_AZIENDALI): gli indirizzi
    su provider generici (gmail, outlook, libero...) non identificano un'azienda e
    non devono rietichettare l'evento.
    """
    if not email or "@" not in email:
        return None
    ind = email.lower().strip()
    dominio = ind.split("@")[-1]
    if dominio not in DOMINI_AZIENDALI:
        return None
    if ind in NOMI_PER_INDIRIZZO and ind not in CASELLE_DI_COLLEGHI:
        return NOMI_PER_INDIRIZZO[ind]
    if dominio in NOMI_PER_DOMINIO:
        return NOMI_PER_DOMINIO[dominio]
    # Dominio aziendale senza nome esplicito: si usa il nome del dominio, mai il
    # nome di un'altra azienda per ripiego.
    return _nome_predefinito(ind)


# Account da non mostrare mai in elenco (nascosti a monte, non solo deselezionati).
ACCOUNT_NASCOSTI = _minuscole(_P.get("account_nascosti"))

# Calendario locale per gli eventi aggiunti a mano dentro l'app.
ACCOUNT_LOCALE = "Eventi personali"
LOCAL_EVENTS_PATH = os.path.join(APP_DIR, "local_events.json")

# Le cartelle calendario con questi nomi (festività, compleanni) sono rumore:
# vengono escluse ovunque. Le festività italiane arrivano dal calendario fisso.
CARTELLE_ESCLUSE = ("festivit", "holiday", "feiertag", "vacances")

# Calendario delle festività italiane, sempre attivo e non elencato tra gli
# account. È il feed pubblico di Google.
FESTIVITA_URL = ("https://calendar.google.com/calendar/ical/"
                 "it.italian%23holiday%40group.v.calendar.google.com/public/basic.ics")
FESTIVITA_ID = "builtin::festivita-it"
FESTIVITA_NOME = "Festività"
COLORE_FESTIVITA = "#8a8a8a"


# Calendari Google letti direttamente via CalDAV, senza passare da Outlook.
# Riusano il token gia' rilasciato a CalDav Synchronizer (vedi google_cal.py):
# serve per un account Google che in Outlook e' configurato come sola posta
# (IMAP), quindi senza calendario. Voci: {"indirizzo": ..., "nome": ...}
CALENDARI_GOOGLE = [
    {"id": "google::" + c["indirizzo"], "indirizzo": c["indirizzo"],
     "nome": c.get("nome") or c["indirizzo"]}
    for c in (_P.get("calendari_google") or [])
    if isinstance(c, dict) and c.get("indirizzo")
]

# Aspetto dell'interfaccia: sigle e loghi delle aziende, calendari "secondari"
# (mostrati sbiaditi), schede della sezione Timesheet. Li legge app.js all'avvio.
INTERFACCIA = dict(_P.get("interfaccia") or {})


# Cartelle "cestino": tutto quello che ci sta dentro e' roba eliminata e non va
# mostrata. I nomi cambiano con la lingua della casella.
CARTELLE_CESTINO = ("posta eliminata", "deleted items", "elementi eliminati",
                    "cestino", "trash", "papierkorb", "corbeille")


def cartella_cestino(nome):
    n = (nome or "").strip().lower()
    return any(n == k for k in CARTELLE_CESTINO)


def cartella_esclusa(nome):
    n = (nome or "").lower()
    return any(k in n for k in CARTELLE_ESCLUSE)


def _nome_predefinito(indirizzo):
    ind = indirizzo.lower()
    if ind in NOMI_PER_INDIRIZZO:
        return NOMI_PER_INDIRIZZO[ind]
    dominio = ind.split("@")[-1]
    if dominio in NOMI_PER_DOMINIO:
        return NOMI_PER_DOMINIO[dominio]
    # Dominio sconosciuto: nome senza estensione, iniziale maiuscola.
    radice = dominio.split(".")[0]
    return radice[:1].upper() + radice[1:] if radice else indirizzo


def account_nascosto(indirizzo):
    return indirizzo.lower() in ACCOUNT_NASCOSTI


# Un colore stabile e distinto per il calendario locale "Altro".
COLORE_LOCALE = "#616161"


def alias_per_account(cfg, indirizzi):
    """Mappa indirizzo -> nome da mostrare. Gli alias scelti a mano vincono."""
    scelti = cfg.get("account_aliases", {})
    out = {}
    for ind in indirizzi:
        manuale = (scelti.get(ind) or "").strip()
        out[ind] = manuale or _nome_predefinito(ind)
    return out


def save_config(cfg):
    with _lock_config:
        scrivi_json(CONFIG_PATH, cfg)


def color_for_account(cfg, username, preferito=None):
    """Restituisce (e assegna se serve) un colore stabile per l'account.

    `preferito` vale solo alla prima assegnazione e solo se nessun altro lo usa:
    serve a dare a un'azienda nuova un colore scelto, invece del primo libero
    della tavolozza.
    """
    colors = cfg.setdefault("account_colors", {})
    if username not in colors:
        used = set(colors.values())
        if preferito and preferito not in used:
            colors[username] = preferito
        else:
            free = next((c for c in COLOR_PALETTE if c not in used), None)
            colors[username] = free or COLOR_PALETTE[len(colors) % len(COLOR_PALETTE)]
        save_config(cfg)
    return colors[username]

