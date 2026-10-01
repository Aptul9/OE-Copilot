"""Lettura dei calendari direttamente da Outlook desktop (COM/MAPI).

Sostituisce il vecchio graph.py: niente MSAL, niente client_id, niente token,
niente registrazione su Azure. Legge quello che Outlook ha gia' sincronizzato in
locale, quindi vede tutti gli account del profilo caricato -- compresi i
calendari Internet sottoscritti via ICS (e' cosi' che entra il calendario Google).

Nota sui filtri: si usa la sintassi DASL con date ISO e non il filtro Jet
"[Start] >= '06/01/2026'", perche' quest'ultimo viene interpretato secondo il
locale di Windows: su una macchina italiana 06/01/2026 diventa 6 gennaio invece
che 1 giugno, e la query restituisce silenziosamente il periodo sbagliato.
"""
import json
import logging
import os
import re
import threading
import time
from datetime import datetime, timedelta

import pythoncom
import win32com.client

import config

log = logging.getLogger(__name__)

OL_FOLDER_CALENDAR = 9      # olFolderCalendar
OL_ITEM_APPOINTMENT = 1     # olAppointmentItem

# Valvole di sicurezza: una serie ricorrente senza fine, letta con
# IncludeRecurrences, puo' generare occorrenze all'infinito se il filtro non
# viene onorato. Meglio troncare che piantare l'interfaccia.
MAX_ITEMS_PER_FOLDER = 2000
MAX_FOLDER_DEPTH = 4

# Alcuni eventi non espongono un campo dedicato per il link della riunione:
# il join URL sta nel corpo del messaggio.
MEETING_URL_RE = re.compile(
    r"https?://(?:[\w.-]*teams\.microsoft\.com|[\w.-]*zoom\.us|meet\.google\.com|"
    r"[\w.-]*webex\.com|meet\.jit\.si|[\w.-]*gotomeeting\.com)/[^\s<>\"']+",
    re.IGNORECASE,
)

_com_lock = threading.Lock()

# Esplorare l'albero completo delle cartelle costa ~17s con una dozzina di
# caselle, mentre interrogare i calendari gia' noti costa ~2s: l'elenco va quindi
# scoperto una volta e poi riaperto per identificativo (GetFolderFromID, diretto).
CACHE_PATH = os.path.join(config.APP_DIR, "calendars_cache.json")
CACHE_TTL_SECONDS = 24 * 3600
# Da alzare quando cambia il significato del contenuto (per esempio l'ordine):
# le cache scritte con una versione precedente vengono buttate.
CACHE_VERSION = 5

# Copia su disco degli eventi gia' letti: permette di mostrare il calendario
# all'istante all'avvio, anche con Outlook ancora chiuso, invece di una griglia
# vuota. I dati vengono poi sostituiti da quelli veri appena Outlook risponde.
EVENTS_CACHE_PATH = os.path.join(config.APP_DIR, "events_cache.json")
MAX_INTERVALLI_IN_CACHE = 12
_cache_lock = threading.Lock()
_cache = None  # [{"entry_id", "store_id", "account", "label"}, ...]
_cache_stamp = 0.0
_cache_caselle = []   # nomi delle caselle di quando si e' esplorato


class OutlookUnavailable(RuntimeError):
    """Outlook non installato o non raggiungibile via COM."""


_ultimo_avvio = 0.0


def _percorso_outlook():
    try:
        import winreg
        chiave = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\OUTLOOK.EXE"
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, chiave) as k:
            return winreg.QueryValueEx(k, None)[0]
    except OSError:
        return None


def _avvia_outlook():
    """Avvia Outlook in modo normale, senza restare in attesa.

    Non si usa la Dispatch per farlo partire: se e' il COM ad avviare Outlook e
    il processo chiamante muore, resta un Outlook a meta' a cui le chiamate
    successive si agganciano bloccandosi per sempre (il COM non ha timeout).
    """
    global _ultimo_avvio
    if time.time() - _ultimo_avvio < 60:
        return  # gia' tentato da poco: non moltiplichiamo i processi
    _ultimo_avvio = time.time()
    exe = _percorso_outlook()
    if not exe:
        return
    try:
        import subprocess
        subprocess.Popen([exe], close_fds=True)
    except OSError:
        pass


# --------------------------------------------------------------------------
# Sessione COM
# --------------------------------------------------------------------------
class _Session:
    """Apre una sessione COM valida per il thread corrente.

    Ogni thread che parla con COM deve chiamare CoInitialize per conto suo:
    l'interfaccia (pywebview) e il servizio promemoria girano su thread diversi.
    """

    def __init__(self, profile=""):
        self.profile = profile or ""
        self.ns = None

    def __enter__(self):
        pythoncom.CoInitialize()
        # Ci si aggancia SOLO a un Outlook gia' in esecuzione. Se non c'e' lo si
        # avvia e si esce con un messaggio: meglio dire "riprova tra un attimo"
        # che restare bloccati a tempo indeterminato su una chiamata COM.
        try:
            app = win32com.client.GetActiveObject("Outlook.Application")
        except pythoncom.com_error:
            pythoncom.CoUninitialize()
            _avvia_outlook()
            raise OutlookUnavailable(
                "Outlook non era in esecuzione: l'ho avviato, mi aggiorno da solo "
                "appena è pronto."
            )
        try:
            self.ns = app.GetNamespace("MAPI")
            # Con piu' di un profilo configurato, e senza un profilo predefinito,
            # MAPI apre la finestra "Scegli profilo" e la chiamata COM resta
            # bloccata finche' qualcuno non risponde. ShowDialog=False la evita.
            try:
                self.ns.Logon(self.profile, "", False, False)
            except pythoncom.com_error:
                pass  # gia' connesso: e' il caso normale, va benissimo
        except pythoncom.com_error as e:
            pythoncom.CoUninitialize()
            raise OutlookUnavailable(
                "Impossibile parlare con Outlook. Verifica che Outlook desktop "
                f"(quello classico, non 'Nuovo Outlook') sia installato. Dettaglio: {e}"
            ) from e
        return self.ns

    def __exit__(self, *exc):
        self.ns = None
        pythoncom.CoUninitialize()
        return False


# --------------------------------------------------------------------------
# Scoperta dei calendari
# --------------------------------------------------------------------------
# Il nome dell'archivio non sempre contiene l'indirizzo: le caselle aggiuntive
# aperte in Outlook si chiamano col nome della persona ("Mario Rossi",
# "Commerciale"). Per riconoscerle davvero serve l'indirizzo del proprietario.
PR_MAILBOX_OWNER_ENTRYID = "http://schemas.microsoft.com/mapi/proptag/0x661B0102"
PR_RECEIVED_BY_SMTP = "http://schemas.microsoft.com/mapi/proptag/0x5D07001F"


def _smtp_archivio(ns, store):
    """Indirizzo della casella a cui appartiene l'archivio, o ""."""
    try:
        nome = store.DisplayName or ""
    except pythoncom.com_error:
        return ""
    if "@" in nome:
        return nome
    try:
        pa = store.PropertyAccessor
        # La proprieta' torna in binario: va convertita in stringa esadecimale,
        # altrimenti GetAddressEntryFromID non la accetta.
        eid = pa.BinaryToString(pa.GetProperty(PR_MAILBOX_OWNER_ENTRYID))
        utente = ns.GetAddressEntryFromID(eid).GetExchangeUser()
        if utente and utente.PrimarySmtpAddress:
            return utente.PrimarySmtpAddress
    except Exception:
        pass
    # Ripiego: l'indirizzo a cui e' stato consegnato un messaggio della casella.
    try:
        items = store.GetDefaultFolder(6).Items
        items.Sort("[ReceivedTime]", True)
        for n, it in enumerate(items):
            if n >= 3:
                break
            try:
                smtp = it.PropertyAccessor.GetProperty(PR_RECEIVED_BY_SMTP)
                if smtp and "@" in smtp:
                    return smtp
            except Exception:
                continue
    except Exception:
        pass
    return ""


def _walk_calendar_folders(folder, account, depth=0):
    """Restituisce ricorsivamente le cartelle di tipo calendario.

    Serve la ricorsione perche' i calendari Internet sottoscritti e i calendari
    secondari NON sono il calendario predefinito dello store: stanno annidati.
    """
    try:
        if folder.DefaultItemType == OL_ITEM_APPOINTMENT:
            yield folder, account
    except pythoncom.com_error:
        return  # cartella non interrogabile: la si salta senza far cadere tutto

    if depth >= MAX_FOLDER_DEPTH:
        return
    try:
        children = list(folder.Folders)
    except pythoncom.com_error:
        return
    for child in children:
        # Un calendario buttato nel cestino non deve tornare a galla nell'app:
        # si smette di scendere appena si incontra la cartella degli eliminati.
        try:
            if config.cartella_cestino(child.Name):
                continue
        except pythoncom.com_error:
            continue
        yield from _walk_calendar_folders(child, account, depth + 1)


def _ordine_profilo(ns):
    """Posizione di ogni casella secondo il profilo Outlook.

    ns.Stores restituisce un ordine interno che non somiglia a nulla di quello
    che l'utente vede; ns.Accounts invece segue l'ordine del profilo, lo stesso
    del riquadro delle cartelle (account principale per primo).
    """
    ordine = {}
    try:
        for i, acc in enumerate(ns.Accounts):
            try:
                ordine[acc.DeliveryStore.DisplayName] = i
            except pythoncom.com_error:
                continue
    except pythoncom.com_error:
        pass
    return ordine


def _discover(ns):
    """Esplorazione completa dell'albero: lenta, da fare il meno possibile."""
    try:
        stores = list(ns.Stores)
    except pythoncom.com_error as e:
        raise OutlookUnavailable(f"Nessuno store Outlook leggibile: {e}") from e

    ordine = _ordine_profilo(ns)

    def posizione(store):
        # Le caselle senza account corrispondente (archivi, .pst) vanno in fondo.
        try:
            return ordine.get(store.DisplayName, len(ordine) + 1)
        except pythoncom.com_error:
            return len(ordine) + 2   # store non interrogabile: ultimissimo

    stores.sort(key=posizione)

    trovati = []
    for store in stores:
        try:
            account = store.DisplayName
            store_id = store.StoreID
            root = store.GetRootFolder()
        except pythoncom.com_error:
            continue  # store offline / non montato
        # Si controlla anche l'indirizzo vero: una casella aggiuntiva si chiama
        # col nome della persona, e per nome non veniva mai riconosciuta.
        smtp = _smtp_archivio(ns, store)
        if (config.account_nascosto(account) or config.store_pubblico(account)
                or (smtp and config.account_nascosto(smtp))):
            continue  # casella esclusa a monte o Cartelle pubbliche
        for folder, acc in _walk_calendar_folders(root, account):
            try:
                name = folder.Name
                if config.cartella_esclusa(name):
                    continue  # festività/compleanni: escluse ovunque
                trovati.append({
                    "entry_id": folder.EntryID,
                    "store_id": store_id,
                    "account": acc,
                    "label": name if name == acc else f"{acc} · {name}",
                })
            except pythoncom.com_error:
                continue
    return trovati


def _firma_caselle(ns):
    """Elenco dei nomi delle caselle montate, per accorgersi dei cambiamenti.

    Costa pochi millisecondi, mentre esplorare tutte le cartelle costa ~17s. Con
    la sola scadenza a 24 ore una casella aggiunta (o tolta) in Outlook restava
    invisibile all'app per un giorno intero, o finche' non si premeva "Ricerca
    calendari nuovi": e' successo davvero, con le riunioni spostate che
    sembravano sparite.
    """
    try:
        return sorted(
            ns.Stores.Item(i).DisplayName
            for i in range(1, ns.Stores.Count + 1))
    except pythoncom.com_error:
        return []


def _read_cache_file():
    try:
        with open(CACHE_PATH, "r", encoding="utf-8") as f:
            data = json.load(f)
        # Una cache di formato vecchio va ignorata: la 1 conteneva l'ordine
        # interno degli Stores invece di quello del profilo Outlook.
        if int(data.get("version") or 1) < CACHE_VERSION:
            return None, 0.0
        return (data.get("entries") or [], float(data.get("stamp") or 0),
                data.get("caselle") or [])
    except (OSError, json.JSONDecodeError, TypeError, ValueError):
        return None, 0.0, []


def _write_cache_file(entries, stamp, caselle=()):
    try:
        # Scrittura atomica: una cache troncata da un'interruzione verrebbe
        # buttata al riavvio, e si ripagherebbero i 17 secondi di esplorazione.
        config.scrivi_json(CACHE_PATH, {
            "version": CACHE_VERSION, "stamp": stamp,
            "entries": entries, "caselle": list(caselle)})
    except OSError:
        pass  # la cache e' un'ottimizzazione: se non si scrive, pazienza


def _iter_calendars(ns, force=False):
    """(folder, account, calendar_id, label) per ogni calendario visibile.

    Usa l'elenco memorizzato quando c'e': riaprire le cartelle per identificativo
    e' immediato, riesplorare l'albero no. Se un identificativo non si risolve
    piu' (calendario rimosso o rinominato) si ricade sull'esplorazione completa.
    """
    global _cache, _cache_stamp, _cache_caselle
    firma = _firma_caselle(ns)
    with _cache_lock:
        entries, stamp, caselle = _cache, _cache_stamp, _cache_caselle
        if entries is None:
            entries, stamp, caselle = _read_cache_file()
        scaduta = (time.time() - stamp) > CACHE_TTL_SECONDS
        cambiate = bool(firma) and bool(caselle) and firma != list(caselle)
        if cambiate:
            log.info("outlook: le caselle sono cambiate, rileggo i calendari")
        if force or not entries or scaduta or cambiate:
            entries = _discover(ns)
            stamp = time.time()
            caselle = firma
            _write_cache_file(entries, stamp, caselle)
        elif firma and not caselle:
            # Cache scritta da una versione precedente, senza la firma: la si
            # annota subito (costa nulla) invece di aspettare la prossima
            # esplorazione, altrimenti il controllo non partirebbe mai.
            caselle = firma
            _write_cache_file(entries, stamp, caselle)
        _cache, _cache_stamp, _cache_caselle = entries, stamp, caselle

    risolti = []
    for e in entries:
        try:
            folder = ns.GetFolderFromID(e["entry_id"], e["store_id"])
        except (pythoncom.com_error, KeyError):
            folder = None
        if folder is None:
            # Elenco non piu' valido: si riparte da una scoperta completa.
            with _cache_lock:
                entries = _discover(ns)
                _cache, _cache_stamp, _cache_caselle = entries, time.time(), firma
                _write_cache_file(entries, _cache_stamp, firma)
            risolti = []
            for e2 in entries:
                try:
                    risolti.append((ns.GetFolderFromID(e2["entry_id"], e2["store_id"]), e2))
                except pythoncom.com_error:
                    continue
            break
        risolti.append((folder, e))

    for folder, e in risolti:
        yield folder, e["account"], e["entry_id"], e["label"]


def list_calendars(cfg, force=False):
    """Elenco dei calendari disponibili, con colore e stato attivo/disattivo.

    force=True rifà la scoperta completa: serve dopo aver sottoscritto in Outlook
    un nuovo calendario (per esempio quello Google via ICS).
    """
    disabled = set(cfg.get("disabled_calendars", []))
    out = []
    with _com_lock, _Session(cfg.get("outlook_profile")) as ns:
        for folder, account, cal_id, label in _iter_calendars(ns, force=force):
            # Leggere il conteggio e' anche la prova che la cartella risponde:
            # se fallisce, quel calendario e' rotto e va segnalato, non nascosto
            # dietro uno zero che sembra "vuoto".
            try:
                count = folder.Items.Count
                stato, stato_msg = "ok", "Leggibile"
            except pythoncom.com_error as e:
                count, stato, stato_msg = 0, "errore", f"Cartella non leggibile ({e})"
            out.append({
                "id": cal_id,
                "account": account,
                "label": label,
                "count": count,
                "color": config.color_for_account(cfg, account),
                "enabled": cal_id not in disabled,
                "stato": stato,
                "stato_msg": stato_msg,
            })
    # Nessun riordino: si tiene l'ordine in cui Outlook espone gli store e le
    # cartelle, cosi' l'elenco rispecchia il riquadro di Outlook invece di
    # presentare un alfabetico che non corrisponde a nulla di familiare.
    alias = config.alias_per_account(cfg, [c["account"] for c in out])
    for c in out:
        c["alias"] = alias.get(c["account"], c["account"])
        # Cartella dedicata a un'azienda (es. un calendario via CalDav): mostra
        # nome e colore dell'azienda invece di quelli della casella che la ospita.
        az = config.azienda_da_cartella(c["label"])
        if az:
            c["alias"] = az
            colore = config.colore_per_azienda(cfg, az)
            if colore:
                c["color"] = colore
    return out


def set_calendar_enabled(cfg, cal_id, enabled):
    disabled = set(cfg.get("disabled_calendars", []))
    if enabled:
        disabled.discard(cal_id)
    else:
        disabled.add(cal_id)
    cfg["disabled_calendars"] = sorted(disabled)
    config.save_config(cfg)


# --------------------------------------------------------------------------
# Copia su disco degli eventi
# --------------------------------------------------------------------------
def _leggi_cache_eventi():
    try:
        with open(EVENTS_CACHE_PATH, "r", encoding="utf-8") as f:
            dati = json.load(f)
        return dati if isinstance(dati, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def _scrivi_cache_eventi(chiave, eventi):
    dati = _leggi_cache_eventi()
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
# Lettura eventi
# --------------------------------------------------------------------------
def _to_naive_local(value):
    """Converte l'orario COM in datetime locale naive.

    Attenzione: pywin32 restituisce un pywintypes.datetime i cui campi (anno,
    ora, minuti) sono gia' l'ora locale corretta, ma con un tzinfo sbagliato
    appiccicato sopra -- qui arriva 'GMT Standard Time' anche se la macchina e'
    su Europe/Rome. Convertire fidandosi di quel fuso (.timestamp(),
    .astimezone()) sposta l'evento di due ore. L'unica lettura corretta e'
    prendere i campi cosi' come sono e scartare il tzinfo.
    """
    if value is None:
        return None
    try:
        return value.replace(tzinfo=None)
    except (AttributeError, TypeError, ValueError):
        return None


def _parse_bound(s):
    """Legge un estremo dell'intervallo in datetime locale naive.

    FullCalendar manda le date con l'offset ('2026-07-20T00:00:00+02:00'),
    mentre gli eventi letti da Outlook sono naive: confrontarli direttamente
    solleva TypeError. Qui l'offset e' autentico, quindi va convertito all'ora
    locale (non scartato come si fa invece con il tzinfo fasullo di pywin32).
    """
    s = s.strip()
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is not None:
        dt = dt.astimezone().replace(tzinfo=None)
    return dt


def _dasl_between(lo, hi):
    """Filtro DASL su dtstart: indipendente dal locale, a differenza di [Start]."""
    fmt = "%Y-%m-%d %H:%M"
    return ('@SQL="urn:schemas:calendar:dtstart" >= \'{}\' AND '
            '"urn:schemas:calendar:dtstart" <= \'{}\''.format(lo.strftime(fmt), hi.strftime(fmt)))


def _restricted_items(folder, lo, hi):
    items = folder.Items
    # L'ordinamento per [Start] e' un prerequisito perche' IncludeRecurrences
    # espanda davvero le occorrenze delle serie ricorrenti.
    items.Sort("[Start]")
    items.IncludeRecurrences = True
    return items.Restrict(_dasl_between(lo, hi))


def crea_bozza_email(cfg, destinatario, cc, oggetto, corpo_html, da_indirizzo=None):
    """Crea una bozza di email e la apre in Outlook (NON la invia).

    Se `da_indirizzo` corrisponde a un account del profilo, la bozza parte da
    quella casella. La firma predefinita viene mantenuta: si carica con
    GetInspector e ci si scrive sopra, cosi' l'utente trova la mail come le sue.
    """
    with _com_lock, _Session(cfg.get("outlook_profile")) as ns:
        app = ns.Application
        mail = app.CreateItem(0)   # olMailItem
        # Firma: forzarla prima di toccare il corpo, poi anteporre il testo.
        try:
            mail.GetInspector
            firma = mail.HTMLBody or ""
        except pythoncom.com_error:
            firma = ""
        mail.To = destinatario or ""
        mail.CC = cc or ""
        mail.Subject = oggetto or ""
        mail.HTMLBody = (corpo_html or "") + firma
        # Invia dalla casella indicata, se la si trova nel profilo.
        if da_indirizzo:
            try:
                for acc in app.Session.Accounts:
                    if (acc.SmtpAddress or "").lower() == da_indirizzo.lower():
                        mail.SendUsingAccount = acc
                        break
            except pythoncom.com_error:
                pass
        mail.Display(False)   # apre la finestra della bozza
        return True


def open_item(cfg, entry_id):
    """Apre l'appuntamento in Outlook (finestra dell'evento)."""
    if not entry_id:
        return False
    with _com_lock, _Session(cfg.get("outlook_profile")) as ns:
        try:
            item = ns.GetItemFromID(entry_id)
            item.Display()
            return True
        except pythoncom.com_error:
            return False


def _extract_meeting_url(item):
    for attr in ("Location", "Body"):
        try:
            text = getattr(item, attr) or ""
        except pythoncom.com_error:
            continue
        match = MEETING_URL_RE.search(text)
        if match:
            return match.group(0).rstrip(">).,;\"'")
    return None


def _organizer_smtp(item):
    """Indirizzo email dell'organizzatore, o "" se non ricavabile.

    Prima prova la via Exchange (PrimarySmtpAddress); per gli organizzatori
    esterni (es. un account Google) quella fallisce e si usa .Address, che
    per una voce di rubrica SMTP è già l'email.
    """
    try:
        org = item.GetOrganizer()
    except (pythoncom.com_error, AttributeError):
        return ""
    if org is None:
        return ""
    try:
        eu = org.GetExchangeUser()
        if eu is not None and eu.PrimarySmtpAddress:
            return eu.PrimarySmtpAddress
    except (pythoncom.com_error, AttributeError):
        pass
    try:
        addr = org.Address or ""
    except (pythoncom.com_error, AttributeError):
        addr = ""
    return addr if "@" in addr else ""


PA_SMTP = "http://schemas.microsoft.com/mapi/proptag/0x39FE001F"   # PR_SMTP_ADDRESS


def _mie_caselle(ns):
    """Indirizzi di tutte le caselle del profilo, in minuscolo."""
    out = set()
    try:
        for acc in ns.Accounts:
            try:
                if acc.SmtpAddress:
                    out.add(acc.SmtpAddress.lower())
            except pythoncom.com_error:
                continue
    except pythoncom.com_error:
        pass
    return out


# L'indirizzo invitato non cambia mai per un dato appuntamento: si legge una
# volta e si tiene a mente (le occorrenze di una serie condividono l'EntryID, e
# la griglia si riaggiorna ogni minuto).
_cache_invitato = {}


def _casella_invitata(item, mie, entry_id=""):
    """Quale TUA casella e' fra gli invitati, o "".

    E' il segnale piu' onesto per capire di che azienda e' una riunione quando
    l'organizzatore e' esterno: se hanno invitato il tuo indirizzo di un'azienda,
    la riunione e' di quell'azienda, anche se Outlook l'ha archiviata nel
    calendario di un'altra (un account IMAP non ha un calendario sul server,
    quindi Outlook ripiega sul calendario predefinito del profilo).
    """
    if entry_id and entry_id in _cache_invitato:
        return _cache_invitato[entry_id]
    try:
        destinatari = item.Recipients
    except pythoncom.com_error:
        return ""
    try:
        for r in destinatari:
            try:
                smtp = r.PropertyAccessor.GetProperty(PA_SMTP)
            except Exception:
                try:
                    smtp = r.Address or ""
                except pythoncom.com_error:
                    continue
            smtp = str(smtp or "").lower()
            if smtp in mie:
                if entry_id:
                    _cache_invitato[entry_id] = smtp
                return smtp
    except pythoncom.com_error:
        return ""
    if entry_id:
        _cache_invitato[entry_id] = ""
    return ""


def _normalize(item, account, color, calendar):
    start = _to_naive_local(item.Start)
    end = _to_naive_local(item.End)
    try:
        entry_id = item.EntryID
    except pythoncom.com_error:
        entry_id = ""
    # Le occorrenze di una serie condividono l'EntryID del master: senza la data
    # nella chiave, il servizio promemoria notificherebbe una riunione ricorrente
    # una volta sola e poi mai piu'.
    uid = f"{entry_id}|{start.isoformat() if start else '?'}"
    try:
        all_day = bool(item.AllDayEvent)
    except pythoncom.com_error:
        all_day = False
    def _safe(attr, default=""):
        try:
            return getattr(item, attr) or default
        except pythoncom.com_error:
            return default
    return {
        "id": uid,
        "account": account,
        "calendar": calendar,
        "color": color,
        "title": _safe("Subject") or "(senza titolo)",
        "start": start.isoformat() if start else None,
        "end": end.isoformat() if end else None,
        "allDay": all_day,
        "location": _safe("Location"),
        "organizer": _safe("Organizer"),
        "organizer_email": _organizer_smtp(item),
        "online_url": _extract_meeting_url(item),
        "web_link": None,  # non applicabile: l'evento si apre in Outlook, non sul web
    }


# --------------------------------------------------------------------------
# Aziende copiate da Power Automate (config.AZIENDE_INOLTRATE)
# --------------------------------------------------------------------------
# Il flusso delle riunioni non da' segni di vita finche' non c'e' una riunione
# nuova: dal solo calendario non si distingue "nessuna riunione" da "flusso
# fermo", e un pallino verde in quel caso mentirebbe. Lo dice invece la posta:
# il flusso gemello inoltra ogni mail della casella d'origine (almeno il
# riepilogo quotidiano) e usa lo stesso collegamento all'account. Se la posta
# inoltrata si ferma, il collegamento e' caduto.
INOLTRI_OGNI_S = 300          # la casella si guarda al massimo ogni 5 minuti
INOLTRI_SOGLIA_ORE = 36       # senza mail inoltrate da piu' di cosi': giallo
_inoltri = {}                 # azienda -> {"letto", "quando", "errore"}


def _store_casella(ns, indirizzo):
    ind = indirizzo.lower()
    for st in ns.Stores:
        try:
            if ind in (st.DisplayName or "").lower():
                return st
        except pythoncom.com_error:
            continue
    return None


def _aggiorna_inoltri(ns):
    """Annota quando e' arrivata l'ultima mail inoltrata, azienda per azienda."""
    adesso = time.time()
    for azienda, d in config.AZIENDE_INOLTRATE.items():
        prec = _inoltri.get(azienda)
        if prec and adesso - prec["letto"] < INOLTRI_OGNI_S:
            continue
        voce = {"letto": adesso, "quando": None, "errore": None,
                "senza_controllo": not d["oggetto_posta"]}
        st = _store_casella(ns, d["via"])
        if st is None:
            voce["errore"] = "Casella %s non trovata in Outlook" % d["via"]
        elif not voce["senza_controllo"]:
            filtro = ("@SQL=\"urn:schemas:httpmail:subject\" LIKE '%s%%'"
                      % d["oggetto_posta"].replace("'", "''"))
            # Posta in arrivo e Posta eliminata: anche una mail letta e buttata
            # e' un segno di vita.
            for id_cartella in (6, 3):
                try:
                    trovate = st.GetDefaultFolder(id_cartella).Items.Restrict(filtro)
                    trovate.Sort("[ReceivedTime]", True)
                    prima = trovate.GetFirst()
                    quando = (_to_naive_local(prima.ReceivedTime)
                              if prima is not None else None)
                except (pythoncom.com_error, AttributeError):
                    log.warning("inoltri: cartella %d di %s non leggibile",
                                id_cartella, d["via"], exc_info=True)
                    continue
                if quando and (voce["quando"] is None or quando > voce["quando"]):
                    voce["quando"] = quando
        _inoltri[azienda] = voce


def stato_inoltro(azienda):
    """Pallino di un'azienda inoltrata: verde solo con posta inoltrata recente."""
    voce = _inoltri.get(azienda)
    if not voce:
        return {"stato": "avviso", "stato_msg": "Non ancora letto"}
    if voce["errore"]:
        return {"stato": "avviso", "stato_msg": voce["errore"]}
    if voce.get("senza_controllo"):
        # Senza "oggetto_posta" in personalizzazione non c'e' modo di sapere se
        # il flusso e' vivo: si dice cosa si e' verificato davvero.
        return {"stato": "ok", "stato_msg": "Copie lette da Outlook; flusso non "
                "verificabile (manca oggetto_posta)"}
    quando = voce["quando"]
    if quando is None:
        via = config.AZIENDE_INOLTRATE.get(azienda, {}).get("via", "")
        return {"stato": "avviso",
                "stato_msg": "Nessuna mail di %s inoltrata in %s: controlla i "
                             "flussi Power Automate" % (azienda, via)}
    ore = (datetime.now() - quando).total_seconds() / 3600
    if ore > INOLTRI_SOGLIA_ORE:
        return {"stato": "avviso",
                "stato_msg": "Nessuna mail di %s inoltrata da %d ore: controlla i "
                             "flussi Power Automate" % (azienda, ore)}
    return {"stato": "ok",
            "stato_msg": "Power Automate attivo, ultima mail inoltrata il %s"
                         % quando.strftime("%d/%m alle %H:%M")}


def calendari_inoltrati(cfg):
    """Voci per la barra laterale, una per azienda inoltrata.

    Non corrispondono a una cartella di Outlook (le copie stanno nel calendario
    di un'altra casella), ma servono lo stesso: interruttore, colore e pallino
    propri, se no l'azienda inoltrata si spegnerebbe solo insieme a quella
    che ospita le copie.
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
            **stato_inoltro(azienda),
        })
    return out


def fetch_all_events(cfg, start_iso, end_iso, allow_cache=True):
    """Eventi uniti da tutti i calendari Outlook attivi nell'intervallo dato.

    Ritorna (eventi, errori, meta). In "meta" si dice se i dati arrivano dalla
    copia su disco (stale) e da quando. Il servizio promemoria passa
    allow_cache=False: notificare partendo da dati vecchi rischierebbe di
    avvisare per riunioni nel frattempo spostate o annullate.
    """
    meta = {"stale": False, "stale_at": None, "outlook_starting": False}
    try:
        start = _parse_bound(start_iso)
        end = _parse_bound(end_iso)
    except (ValueError, AttributeError) as e:
        return [], [f"Intervallo non valido: {e}"], meta
    chiave_cache = f"{start.isoformat()}|{end.isoformat()}"

    # Il filtro lavora su dtstart: allargando il bordo inferiore si recuperano
    # anche gli eventi iniziati prima della finestra ma ancora in corso.
    lo = start - timedelta(days=1)
    disabled = set(cfg.get("disabled_calendars", []))
    events, errors = [], []
    # Stesso evento presente in piu' calendari dello stesso account:
    # chiave -> posizione in `events`.
    seen = {}

    # Per ogni azienda (nome mostrato) si annota se ha almeno un calendario
    # acceso: serve a spegnere/accendere per AZIENDA e non per cartella. Una
    # riunione di un'azienda archiviata da Outlook nel calendario di un'altra
    # deve seguire l'interruttore della sua azienda, non della casella.
    aziende = {}
    alias_casella = config.alias_per_account(cfg, [])

    try:
        with _com_lock, _Session(cfg.get("outlook_profile")) as ns:
            mie = _mie_caselle(ns)
            for folder, account, cal_id, label in _iter_calendars(ns):
                if account not in alias_casella:
                    alias_casella.update(config.alias_per_account(cfg, [account]))
                azienda = (config.azienda_da_cartella(label)
                           or alias_casella.get(account, account))
                aziende[azienda] = aziende.get(azienda, False) or (cal_id not in disabled)
                # Anche i calendari spenti si leggono: possono contenere riunioni
                # di un'ALTRA azienda (Outlook archivia gli inviti di un account
                # IMAP nel calendario predefinito) e devono restare visibili se la loro
                # azienda e' accesa. A scartare ci pensa il filtro per azienda,
                # a valle della riclassificazione.
                color = config.color_for_account(cfg, account)
                try:
                    restricted = _restricted_items(folder, lo, end)
                except pythoncom.com_error as e:
                    errors.append(f"{label}: {e}")
                    continue
                try:
                    for n, item in enumerate(restricted):
                        if n >= MAX_ITEMS_PER_FOLDER:
                            errors.append(f"{label}: troppi eventi, elenco troncato")
                            break
                        ev = _normalize(item, account, color, label)
                        # Il filtro guarda solo l'inizio: qui si tiene davvero
                        # solo cio' che si sovrappone alla finestra richiesta.
                        if not ev["start"]:
                            continue
                        ev_start = datetime.fromisoformat(ev["start"])
                        ev_end = datetime.fromisoformat(ev["end"]) if ev["end"] else ev_start
                        if not (ev_end > start and ev_start < end):
                            continue
                        # Copia messa qui da un flusso (le riunioni di
                        # un'azienda inoltrata, copiate in questa casella): e' di
                        # quell'azienda, e il prefisso nel titolo non serve
                        # piu' perche' lo dice gia' l'etichetta.
                        inoltrata, titolo = config.azienda_da_titolo(ev["title"])
                        if inoltrata:
                            ev["title"] = titolo
                            ev["inoltrata_da"] = inoltrata
                        # Se ne' la cartella ne' l'organizzatore dicono di
                        # che azienda e', si guarda a quale casella tua era
                        # indirizzato l'invito. Solo in questo caso: leggere i
                        # destinatari di ogni evento costerebbe troppo.
                        if not (inoltrata
                                or config.azienda_da_cartella(label)
                                or config.azienda_da_email(ev.get("organizer_email"))):
                            ev["invitato_come"] = _casella_invitata(
                                item, mie, ev["id"].split("|", 1)[0])
                        key = (account, ev["title"], ev["start"], ev["end"])
                        if key in seen:
                            # La stessa riunione arrivata sia come invito vero
                            # sia come copia del flusso: vince l'invito, che ha
                            # il link per partecipare e l'organizzatore reale.
                            if events[seen[key]].get("inoltrata_da") and not inoltrata:
                                events[seen[key]] = ev
                            continue
                        seen[key] = len(events)
                        events.append(ev)
                except pythoncom.com_error as e:
                    errors.append(f"{label}: lettura interrotta ({e})")
            # A sessione gia' aperta costa poco: dice se i flussi che copiano
            # le riunioni delle aziende inoltrate sono vivi (_aggiorna_inoltri).
            try:
                _aggiorna_inoltri(ns)
            except Exception:
                log.exception("inoltri: controllo della posta inoltrata fallito")
    except OutlookUnavailable as e:
        # Outlook non c'e': invece di una griglia vuota si mostra l'ultima
        # lettura salvata, dichiarandola come non aggiornata.
        if allow_cache:
            salvato = _leggi_cache_eventi().get(chiave_cache)
            if salvato:
                meta["stale"] = True
                meta["stale_at"] = datetime.fromtimestamp(
                    salvato.get("stamp", 0)).isoformat(timespec="seconds")
                meta["outlook_starting"] = True
                return salvato.get("eventi", []), [str(e)], meta
        meta["outlook_starting"] = True
        return [], [str(e)], meta

    events.sort(key=lambda e: e["start"] or "")
    alias = config.alias_per_account(cfg, {e["account"] for e in events})
    # Colore per azienda ricavato dai TUOI account già colorati: così un evento
    # riclassificato su un'azienda riusa lo stesso colore della tua casella di
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
    meta["aziende"] = aziende
    if allow_cache:
        _scrivi_cache_eventi(chiave_cache, events)
    return events, errors, meta

