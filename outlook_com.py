"""Apertura di un appuntamento in Outlook desktop (COM/MAPI): il pulsante "Apri in Outlook".

Le caselle non si leggono da qui ma dall'agente remoto (agente_remoto.py): Outlook sul PC serve solo per
trovare un appuntamento per ora d'inizio e titolo e aprirne la finestra.

Nota sui filtri: si usa la sintassi DASL con date ISO e non il filtro Jet
"[Start] >= '06/01/2026'", perché quest'ultimo viene interpretato secondo il
locale di Windows: su una macchina italiana 06/01/2026 diventa 6 gennaio invece
che 1 giugno, e la query restituisce silenziosamente il periodo sbagliato.
"""
import logging
import threading
import time
from datetime import timedelta

import pythoncom
import win32com.client

import caselle
import config

log = logging.getLogger(__name__)

OL_FOLDER_CALENDAR = 9      # olFolderCalendar

# Valvola di sicurezza: una serie ricorrente senza fine, letta con
# IncludeRecurrences, può generare occorrenze all'infinito se il filtro non
# viene onorato. Meglio troncare che piantare l'interfaccia.
MAX_ITEMS_PER_FOLDER = 2000

_com_lock = threading.Lock()


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


def _store_casella(ns, indirizzo):
    ind = indirizzo.lower()
    for st in ns.Stores:
        try:
            if ind in (st.DisplayName or "").lower():
                return st
        except pythoncom.com_error:
            continue
    return None


def _trova_per_inizio(ns, account, inizio, titolo):
    """L'appuntamento di una casella che inizia in quel minuto, o None.

    Fra quelli che iniziano in quel minuto (ricorrenze espanse) si prende quello
    col titolo giusto, confrontato senza il prefisso delle copie inoltrate, che
    l'app toglie; se nessun titolo combacia, il primo.

    Il filtro di Outlook legge le date in UTC, non in ora locale: una finestra
    stretta attorno all'ora cercata guarderebbe una o due ore piu' in la'. Si
    chiede quindi un giorno per lato e l'ora esatta si confronta qui.
    """
    store = _store_casella(ns, account or "")
    if store is None:
        return None
    atteso = (titolo or "").strip().lower()
    primo = None
    trovati = _restricted_items(store.GetDefaultFolder(OL_FOLDER_CALENDAR),
                                inizio - timedelta(days=1), inizio + timedelta(days=1))
    for n, item in enumerate(trovati):
        if n >= MAX_ITEMS_PER_FOLDER:
            break
        quando = _to_naive_local(item.Start)
        if quando is None or abs(quando - inizio) >= timedelta(minutes=1):
            continue
        if config.azienda_da_titolo(item.Subject or "")[1].strip().lower() == atteso:
            return item
        primo = primo or item
    return primo


def open_item_by(cfg, account, start_iso, titolo):
    """Apre l'appuntamento di una casella sapendo solo quando inizia e come si chiama.

    Serve per gli eventi che non arrivano da qui (agente remoto) e quindi non
    hanno un EntryID.
    """
    try:
        inizio = caselle.parse_bound(start_iso)
    except (TypeError, ValueError, AttributeError):
        return False
    with _com_lock, _Session(cfg.get("outlook_profile")) as ns:
        try:
            item = _trova_per_inizio(ns, account, inizio, titolo)
            if item is None:
                return False
            item.Display()
            return True
        except pythoncom.com_error:
            log.warning("open_item_by: ricerca in %s non riuscita", account, exc_info=True)
            return False
