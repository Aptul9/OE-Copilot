"""OE Copilot - finestra desktop che unisce i calendari Outlook di più account."""
import logging
import os
import sys
import time
import webbrowser

import webview

import caselle
import config
import eventi as raccolta_eventi
import google_cal
import ics_feed
import local_events
import outlook_com
import reminders
import teams_link

# Risorse (interfaccia, icona) dal pacchetto; log e dati scrivibili in DATA_DIR.
WEB_DIR = os.path.join(config.RESOURCE_DIR, "web")
ICON_PATH = os.path.join(config.RESOURCE_DIR, "icon.ico")
DEBUG_LOG = os.path.join(config.DATA_DIR, "debug.log")

# Il log si APPENDE e ruota da solo. Prima veniva azzerato a ogni avvio
# (filemode="w"): bastava un doppio clic di troppo sull'icona per cancellare la
# cronologia della copia in esecuzione, proprio mentre serviva per capire un
# guasto. Con la rotazione il file non cresce comunque all'infinito.
try:
    from logging.handlers import RotatingFileHandler
    _h = RotatingFileHandler(DEBUG_LOG, maxBytes=2_000_000, backupCount=2,
                             encoding="utf-8")
    # Il numero di processo nel log: se per sbaglio partono due copie (succede
    # facendo doppio clic su avvia.pyw invece che sul collegamento) le loro righe
    # si mescolano nello stesso file e diventano illeggibili.
    _h.setFormatter(logging.Formatter(
        "%(asctime)s [pid %(process)d/%(threadName)s] %(message)s"))
    logging.basicConfig(level=logging.INFO, handlers=[_h])
except OSError:
    logging.basicConfig(level=logging.INFO)
# requests/urllib3 in DEBUG riempiono il log di righe di connessione e
# seppelliscono le nostre: qui interessano solo i loro problemi veri.
logging.getLogger("urllib3").setLevel(logging.WARNING)
log = logging.getLogger("calendarhub")
log.info("--- avvio ---")



class Api:
    """Metodi richiamabili dall'interfaccia (JavaScript) tramite pywebview.api.*"""

    def __init__(self):
        self._maximized = False
        self._integrazioni_viste = False
        self._rect_normale = None   # bounds prima di massimizzare
        # ATTENZIONE al trattino basso: pywebview espone a JavaScript i metodi di
        # questa classe camminandone ricorsivamente gli attributi, e salta solo
        # quelli che iniziano con "_". Se la finestra fosse un attributo pubblico,
        # il serializzatore ci scenderebbe dentro (finestra -> Form WinForms ->
        # AccessibilityObject.Bounds.Empty.Empty...) andando in ricorsione infinita:
        # window.pywebview.api non verrebbe mai creato e la pagina resterebbe vuota.
        self._window = None

    # ---- Configurazione ------------------------------------------------
    def get_config(self):
        return config.load_config()

    def save_config(self, new_cfg):
        cfg = config.load_config()
        cfg.update(new_cfg or {})
        config.save_config(cfg)
        return {"ok": True}

    # ---- Calendari -----------------------------------------------------
    def log_js(self, msg):
        """Ponte per far arrivare gli errori JavaScript nel log di Python."""
        log.error("JS: %s", msg)
        return {"ok": True}

    def log_ui(self, msg):
        """Traccia di avvio riuscito dell'interfaccia (la cerca verifica.py)."""
        log.info("UI: %s", msg)
        return {"ok": True}

    def get_calendars(self, force=False):
        """Calendari delle caselle + quelli sottoscritti da link ICS."""
        t0 = time.time()
        log.info("get_calendars: inizio (force=%s)", force)
        cfg = config.load_config()
        # Al primo giro (e a ogni "aggiorna") le integrazioni si verificano sul
        # serio: il pallino di stato dev'essere un fatto, non l'eco di una
        # lettura andata bene mezz'ora fa.
        try:
            google_cal.stato_integrazioni(
                verifica=bool(force) or not self._integrazioni_viste)
            self._integrazioni_viste = True
        except Exception:
            log.exception("get_calendars: verifica integrazioni fallita")
        def _ordina(lista):
            # Ordine scelto dall'utente per nome mostrato; stabile per il resto.
            return sorted(lista, key=lambda c: config.ordine_calendario(
                c.get("alias") or c.get("account") or ""))
        cals, errore = [], None
        try:
            cals = raccolta_eventi.calendari_caselle(cfg, force=bool(force))
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE as e:
            log.warning("get_calendars: caselle non disponibili: %s", e)
            errore = str(e)
        except Exception as e:
            log.exception("get_calendars: eccezione")
            errore = str(e)
        # Dopo le caselle, non prima: il pallino di un'azienda inoltrata dipende
        # da com'e' andata la lettura della casella che ne ospita le copie.
        # I calendari da link e locali restano leggibili anche senza le caselle.
        extra = raccolta_eventi.calendari_extra(cfg)
        log.info("get_calendars: %d delle caselle + %d extra in %.1fs",
                 len(cals), len(extra), time.time() - t0)
        return {"calendars": _ordina(cals + extra), "error": errore}

    def set_calendar_enabled(self, cal_id, enabled):
        caselle.set_calendar_enabled(config.load_config(), cal_id, bool(enabled))
        return {"ok": True}

    def set_account_enabled(self, account, enabled):
        """Accende/spegne in blocco tutti i calendari di un account."""
        cfg = config.load_config()
        # Vanno considerate TUTTE le sorgenti, non solo Outlook e i feed ICS:
        # altrimenti le voci che arrivano da Google o dal calendario locale
        # restano accese qualunque cosa si clicchi.
        try:
            outlook = raccolta_eventi.calendari_caselle(cfg)
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE:
            outlook = []   # senza le caselle si spengono comunque le altre voci
        tutti = outlook + raccolta_eventi.calendari_extra(cfg)
        for c in tutti:
            if c["account"] == account:
                caselle.set_calendar_enabled(cfg, c["id"], bool(enabled))
        return {"ok": True}

    def set_azienda_enabled(self, azienda, enabled):
        """Accende/spegne TUTTI i calendari di un'azienda, in qualunque casella.

        La stessa azienda puo' avere piu' calendari in posti diversi (3D
        Informatica sta sia nel calendario Google sia in una cartella di
        Outlook). Nella barra laterale e' una voce sola: deve comandarli tutti,
        altrimenti se ne spegne uno e sparisce meta' delle riunioni senza che si
        capisca perche'.
        """
        cfg = config.load_config()
        try:
            outlook = raccolta_eventi.calendari_caselle(cfg)
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE:
            outlook = []
        tutti = outlook + raccolta_eventi.calendari_extra(cfg)
        spenti = set(cfg.get("disabled_calendars", []))
        toccati = 0
        for c in tutti:
            if (c.get("alias") or c.get("account")) != azienda:
                continue
            toccati += 1
            if enabled:
                spenti.discard(c["id"])
            else:
                spenti.add(c["id"])
        cfg["disabled_calendars"] = sorted(spenti)
        config.save_config(cfg)
        log.info("set_azienda_enabled: %s -> %s (%d calendari)",
                 azienda, "acceso" if enabled else "spento", toccati)
        return {"ok": True, "calendari": toccati}

    def mostra_solo_azienda(self, azienda):
        """Lascia accesa una sola azienda, o tutte se `azienda` è vuota.

        In un colpo solo: spegnere le altre una per una dalla barra laterale
        vorrebbe un clic e una rilettura per ciascuna.
        """
        self._accendi_solo(lambda nome: not azienda or nome == azienda)
        log.info("mostra_solo_azienda: %s", azienda or "tutte")
        return {"ok": True}

    def mostra_solo_gruppo(self, gruppo):
        """Lascia accese le aziende di un gruppo (config "gruppi_calendari"), spegne le altre."""
        membri = config.membri_gruppo(config.load_config(), gruppo)
        if membri is None:
            return {"ok": False, "error": "Gruppo non trovato"}
        self._accendi_solo(lambda nome: nome in membri)
        log.info("mostra_solo_gruppo: %s (%d aziende)", gruppo, len(membri))
        return {"ok": True}

    def _accendi_solo(self, accesa):
        """Accende le aziende per cui `accesa(nome)` è vera e spegne tutte le altre."""
        cfg = config.load_config()
        try:
            outlook = raccolta_eventi.calendari_caselle(cfg)
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE:
            outlook = []
        tutti = outlook + raccolta_eventi.calendari_extra(cfg)
        spenti = set(cfg.get("disabled_calendars", []))
        for c in tutti:
            if accesa(c.get("alias") or c.get("account")):
                spenti.discard(c["id"])
            else:
                spenti.add(c["id"])
        cfg["disabled_calendars"] = sorted(spenti)
        config.save_config(cfg)

    def set_account_alias(self, account, alias):
        """Nome da mostrare per un account. Vuoto = torna a quello automatico."""
        cfg = config.load_config()
        aliases = cfg.setdefault("account_aliases", {})
        prima = config.alias_per_account(cfg, [account])[account]
        if (alias or "").strip():
            aliases[account] = alias.strip()
        else:
            aliases.pop(account, None)
        config.rinomina_nei_gruppi(cfg, prima, config.alias_per_account(cfg, [account])[account])
        config.save_config(cfg)
        return {"ok": True}

    # ---- Calendari da link ICS ----------------------------------------
    def add_ics_calendar(self, name, url):
        try:
            feed = ics_feed.add_feed(config.load_config(), name, url)
            log.info("add_ics_calendar: aggiunto %s", feed["name"])
            return {"ok": True, "feed": feed}
        except ValueError as e:
            return {"ok": False, "error": str(e)}
        except Exception as e:
            log.exception("add_ics_calendar: eccezione")
            return {"ok": False, "error": str(e)}

    def remove_ics_calendar(self, feed_id):
        ics_feed.remove_feed(config.load_config(), feed_id)
        return {"ok": True}

    # ---- Eventi locali (calendario "Altro") ----------------------------
    def add_local_event(self, dati):
        try:
            local_events.add_event(config.load_config(), dati or {})
            return {"ok": True}
        except ValueError as e:
            return {"ok": False, "error": str(e)}
        except Exception as e:
            log.exception("add_local_event: eccezione")
            return {"ok": False, "error": str(e)}

    def delete_local_event(self, uid):
        local_events.delete_event(config.load_config(), uid)
        return {"ok": True}

    def open_in_outlook(self, event_id):
        """Apre in Outlook l'evento dato il suo id (entry_id|inizio)."""
        entry_id = (event_id or "").split("|", 1)[0]
        try:
            ok = outlook_com.open_item(config.load_config(), entry_id)
            return {"ok": ok}
        except Exception as e:
            log.exception("open_in_outlook: eccezione")
            return {"ok": False, "error": str(e)}

    def apri_in_outlook_remoto(self, account, start_iso, titolo):
        """Apre in Outlook un evento letto dall'agente remoto.

        Quegli eventi non portano un EntryID di Outlook: lo si cerca nella
        casella per ora d'inizio e titolo. Serve Outlook aperto su questo PC,
        che in questa modalita' non e' piu' necessario per il calendario.
        """
        try:
            ok = outlook_com.open_item_by(config.load_config(), account, start_iso, titolo)
            return {"ok": ok, "error": None if ok else "Evento non trovato in Outlook"}
        except outlook_com.OutlookUnavailable as e:
            return {"ok": False, "error": str(e)}
        except Exception as e:
            log.exception("apri_in_outlook_remoto: eccezione")
            return {"ok": False, "error": str(e)}

    def apri_in_teams(self, online_url, account):
        """Apre Teams sul calendario dell'account dell'evento e poi la riunione."""
        try:
            res = teams_link.apri(online_url, account)
            log.info("apri_in_teams: account=%s -> %s", account, res.get("calendario"))
            return res
        except Exception as e:
            log.exception("apri_in_teams: eccezione")
            return {"ok": False, "error": str(e)}

    # ---- Eventi --------------------------------------------------------
    def get_events(self, start_iso, end_iso):
        t0 = time.time()
        log.info("get_events: inizio %s -> %s", start_iso, end_iso)
        try:
            cfg = config.load_config()
            events, errors, meta = raccolta_eventi.raccogli(cfg, start_iso, end_iso)
            log.info("get_events: %d eventi in %.1fs (stale=%s, errori: %s)",
                     len(events), time.time() - t0, meta["stale"], errors)
            return {"events": events, "errors": errors, **meta}
        except Exception as e:
            log.exception("get_events: eccezione")
            return {"events": [], "errors": [str(e)],
                    "stale": False, "stale_at": None, "outlook_starting": False}

    def get_integrazioni(self, verifica=False):
        """Stato dei calendari collegati direttamente (per ora i calendari Google)."""
        try:
            voci = google_cal.stato_integrazioni(verifica=bool(verifica))
            log.info("get_integrazioni: %s",
                     [(v["nome"], v.get("stato")) for v in voci])
            return {"integrazioni": voci}
        except Exception as e:
            log.exception("get_integrazioni: eccezione")
            return {"integrazioni": [], "error": str(e)}

    # ---- Utility -------------------------------------------------------
    def open_external(self, url):
        """Apre il link SEMPRE in Chrome (con fallback al browser di sistema)."""
        if not url:
            return {"ok": True}
        chrome = _percorso_chrome()
        try:
            if chrome:
                import subprocess
                subprocess.Popen([chrome, url], close_fds=True)
            else:
                webbrowser.open(url)
        except Exception:
            log.exception("open_external")
            webbrowser.open(url)
        return {"ok": True}

    def get_personalizzazione(self):
        """Impostazioni dell'interfaccia di chi usa l'app (sigle, loghi, schede).

        Arrivano da personalizzazione.json: nel codice non ci sono nomi di
        aziende. Con il file assente l'interfaccia parte con i valori neutri.
        """
        return {"interfaccia": config.INTERFACCIA,
                "estensioni": sorted(_ESTENSIONI_CARICATE),
                # Da dove arrivano le caselle: l'interfaccia adatta i testi.
                "sorgente": "remota" if raccolta_eventi.agente_remoto.attivo() else "outlook"}

    def test_notification(self):
        return {"message": reminders.send_test_notification(config.load_config())}

    # ---- Avvio automatico con Windows ----------------------------------
    def get_autostart(self):
        return {"enabled": _autostart_attivo()}

    def set_autostart(self, enabled):
        try:
            _imposta_autostart(bool(enabled))
            return {"ok": True, "enabled": bool(enabled)}
        except Exception as e:
            log.exception("set_autostart")
            return {"ok": False, "error": str(e)}

    # ---- Controlli finestra (barra del titolo nostra) ------------------
    def win_minimize(self):
        try:
            self._window.minimize()
        except Exception:
            log.exception("win_minimize")
        return {"ok": True}

    def win_maximize_toggle(self):
        """Massimizza all'area di lavoro (senza coprire la barra applicazioni)
        e ripristina. Non si usa lo stato Maximized nativo perché su una finestra
        senza cornice coprirebbe la taskbar."""
        import ctypes
        from ctypes import wintypes
        u = ctypes.windll.user32
        hwnd = _hwnd_processo()
        if not hwnd:
            return {"maximized": self._maximized}
        SWP = 0x0004 | 0x0010 | 0x0020  # NOZORDER | NOACTIVATE | FRAMECHANGED
        if not self._maximized:
            r = wintypes.RECT()
            u.GetWindowRect(hwnd, ctypes.byref(r))
            self._rect_normale = (r.left, r.top, r.right - r.left, r.bottom - r.top)
            x, y, w, h = _area_lavoro(hwnd)
            u.SetWindowPos(hwnd, 0, x, y, w, h, SWP)
            self._maximized = True
        else:
            if self._rect_normale:
                x, y, w, h = self._rect_normale
                u.SetWindowPos(hwnd, 0, x, y, w, h, SWP)
            self._maximized = False
        return {"maximized": self._maximized}

    def win_resize(self, edge):
        """Avvia il ridimensionamento nativo dal bordo indicato.

        Con la finestra senza cornice, WebView2 copre tutto il client e i bordi
        non ricevono WM_NCHITTEST: il trascinamento non parte da solo. Qui la
        pagina rileva il bordo e chiama questo metodo, che chiede a Windows di
        avviare il suo loop di resize (ReleaseCapture + WM_NCLBUTTONDOWN con il
        codice del bordo) — lo stesso meccanismo delle finestre normali."""
        import ctypes
        if self._maximized:
            return {"ok": False}   # da massimizzata non si ridimensiona
        codici = {"l": 10, "r": 11, "t": 12, "tl": 13, "tr": 14,
                  "b": 15, "bl": 16, "br": 17}   # HTLEFT..HTBOTTOMRIGHT
        ht = codici.get(edge)
        hwnd = _hwnd_processo()
        if not ht or not hwnd:
            return {"ok": False}
        u = ctypes.windll.user32
        u.ReleaseCapture()
        u.SendMessageW(hwnd, 0x00A1, ht, 0)   # WM_NCLBUTTONDOWN
        return {"ok": True}

    def win_close(self):
        try:
            self._window.destroy()
        except Exception:
            log.exception("win_close")
        return {"ok": True}

    def win_nudge(self):
        """Forza il ridisegno del WebView2 con un micro-ridimensionamento.
        Serve dopo un ripristino da icona, quando la finestra senza cornice a
        volte resta bianca finché non cambia dimensione."""
        import ctypes
        from ctypes import wintypes
        u = ctypes.windll.user32
        hwnd = _hwnd_processo()
        if not hwnd:
            return {"ok": False}
        r = wintypes.RECT()
        u.GetWindowRect(hwnd, ctypes.byref(r))
        w, h = r.right - r.left, r.bottom - r.top
        SWP = 0x0002 | 0x0004 | 0x0010  # NOMOVE | NOZORDER | NOACTIVATE
        u.SetWindowPos(hwnd, 0, 0, 0, w, h + 1, SWP)
        u.SetWindowPos(hwnd, 0, 0, 0, w, h, SWP)
        return {"ok": True}

    # ---- interno -------------------------------------------------------
    def _notify_js(self, fn, payload):
        if not self._window:
            return
        import json
        try:
            self._window.evaluate_js(f"window.{fn} && window.{fn}({json.dumps(payload)})")
        except Exception:
            pass


# Estensioni personali: funzioni in piu' scritte per una persona sola (per
# esempio la mail mensile di un timesheet), che non stanno nel repository. Si
# mettono nella cartella "personale" accanto all'app: personale/estensione.py per
# la parte Python (METODI_API = {"nome": funzione}) e web/personale/estensione.js
# per l'interfaccia. Se la cartella non c'e', l'app funziona senza.
_ESTENSIONI_CARICATE = set()


def _carica_estensioni():
    try:
        from personale import estensione
    except ImportError:
        return
    except Exception:
        log.exception("estensione personale non caricata")
        return
    for nome, funzione in (getattr(estensione, "METODI_API", None) or {}).items():
        if hasattr(Api, nome):
            log.warning("estensione: il metodo %s esiste gia', ignorato", nome)
            continue
        setattr(Api, nome, funzione)
        _ESTENSIONI_CARICATE.add(nome)
    log.info("estensione personale caricata: %s", sorted(_ESTENSIONI_CARICATE))


_carica_estensioni()


def _percorso_chrome():
    """Percorso di chrome.exe, o None se non trovato."""
    import winreg
    for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
        try:
            with winreg.OpenKey(hive,
                    r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe") as k:
                p = winreg.QueryValueEx(k, None)[0]
                if p and os.path.exists(p):
                    return p
        except OSError:
            continue
    for p in (
        os.path.join(os.environ.get("PROGRAMFILES", r"C:\Program Files"),
                     r"Google\Chrome\Application\chrome.exe"),
        os.path.join(os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)"),
                     r"Google\Chrome\Application\chrome.exe"),
        os.path.join(os.environ.get("LOCALAPPDATA", ""),
                     r"Google\Chrome\Application\chrome.exe"),
    ):
        if p and os.path.exists(p):
            return p
    return None


# --- Avvio automatico all'accensione (chiave di registro "Run") ------------
_AUTOSTART_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
_AUTOSTART_NOME = "OECopilot"


def _comando_avvio():
    """Comando che Windows deve lanciare all'accensione per avviare l'app."""
    if getattr(sys, "frozen", False):
        return f'"{sys.executable}"'
    pyw = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "avvia.pyw")
    return f'"{pyw}" "{script}"'


def _autostart_attivo():
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, _AUTOSTART_KEY) as k:
            winreg.QueryValueEx(k, _AUTOSTART_NOME)
            return True
    except OSError:
        return False


def _imposta_autostart(attivo):
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, _AUTOSTART_KEY, 0,
                        winreg.KEY_SET_VALUE) as k:
        if attivo:
            winreg.SetValueEx(k, _AUTOSTART_NOME, 0, winreg.REG_SZ, _comando_avvio())
        else:
            try:
                winreg.DeleteValue(k, _AUTOSTART_NOME)
            except OSError:
                pass


def _hwnd_processo():
    """Handle della finestra top-level di questo processo (Win32, thread-safe)."""
    import ctypes
    from ctypes import wintypes
    u = ctypes.windll.user32
    mio = os.getpid()
    out = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def cb(hwnd, _):
        pid = wintypes.DWORD()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if pid.value == mio and u.IsWindowVisible(hwnd) and u.GetWindow(hwnd, 4) == 0:
            out.append(hwnd)
            return False
        return True

    u.EnumWindows(cb, 0)
    return out[0] if out else None


def _area_lavoro(hwnd):
    """Rettangolo dell'area di lavoro (schermo meno la barra applicazioni)."""
    import ctypes
    from ctypes import wintypes

    class MONITORINFO(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.DWORD), ("rcMonitor", wintypes.RECT),
                    ("rcWork", wintypes.RECT), ("dwFlags", wintypes.DWORD)]

    u = ctypes.windll.user32
    hmon = u.MonitorFromWindow(hwnd, 2)  # MONITOR_DEFAULTTONEAREST
    mi = MONITORINFO()
    mi.cbSize = ctypes.sizeof(MONITORINFO)
    u.GetMonitorInfoW(hmon, ctypes.byref(mi))
    r = mi.rcWork
    return r.left, r.top, r.right - r.left, r.bottom - r.top


# --- Una sola copia in esecuzione ------------------------------------------
# Due copie insieme fanno danni concreti: due servizi promemoria (notifiche
# doppie) e due processi che scrivono lo stesso debug.log azzerandolo a vicenda.
# Il mutex di Windows e' il modo standard di accorgersene; l'handle va tenuto in
# vita per tutta la durata del processo, altrimenti il nome si libera subito.
_MUTEX_NOME = r"Local\OECopilot-istanza-unica"
_mutex = None
ERROR_ALREADY_EXISTS = 183


def _istanza_gia_attiva():
    try:
        import ctypes
        global _mutex
        # `windll.kernel32.GetLastError()` non e' affidabile con ctypes (l'errore
        # puo' essere gia' stato sovrascritto): serve use_last_error e
        # ctypes.get_last_error(). Con la lettura sbagliata il controllo non
        # scattava e due copie dell'app giravano insieme, scrivendo sullo stesso
        # log e litigando sui calendari.
        k = ctypes.WinDLL("kernel32", use_last_error=True)
        k.CreateMutexW.restype = ctypes.c_void_p
        k.CreateMutexW.argtypes = [ctypes.c_void_p, ctypes.c_bool, ctypes.c_wchar_p]
        _mutex = k.CreateMutexW(None, False, _MUTEX_NOME)
        errore = ctypes.get_last_error()
        log.info("istanza singola: handle=%s errore=%s", bool(_mutex), errore)
        return errore == ERROR_ALREADY_EXISTS
    except Exception:
        log.exception("controllo istanza singola fallito")
        return False   # nel dubbio si parte: meglio due finestre che nessuna


def _porta_in_primo_piano_altra_istanza():
    """Mostra la finestra della copia gia' aperta, invece di non fare nulla."""
    import ctypes
    from ctypes import wintypes
    u, k = ctypes.windll.user32, ctypes.windll.kernel32
    mio_pid = os.getpid()
    mio_exe = (sys.executable or "").lower()
    trovata = []

    def percorso_di(pid):
        h = k.OpenProcess(0x1000, False, pid)   # PROCESS_QUERY_LIMITED_INFORMATION
        if not h:
            return ""
        try:
            buf = ctypes.create_unicode_buffer(1024)
            n = wintypes.DWORD(1024)
            if k.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(n)):
                return buf.value.lower()
        finally:
            k.CloseHandle(h)
        return ""

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def cb(hwnd, _):
        pid = wintypes.DWORD()
        u.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if (pid.value != mio_pid and u.IsWindowVisible(hwnd)
                and u.GetWindow(hwnd, 4) == 0 and percorso_di(pid.value) == mio_exe):
            trovata.append(hwnd)
            return False
        return True

    u.EnumWindows(cb, 0)
    if trovata:
        u.ShowWindow(trovata[0], 9)      # SW_RESTORE
        u.SetForegroundWindow(trovata[0])


def _identifica_app_a_windows():
    """Dà all'app un'identità propria nella barra delle applicazioni.

    Senza questo, Windows raggruppa la finestra sotto pythonw.exe e ne mostra
    l'icona (il serpente di Python) ignorando quella della finestra.
    """
    try:
        import ctypes
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID(
            "OECopilot.App")
    except Exception:
        pass  # solo estetica: se fallisce, l'app parte lo stesso


def main():
    log.info("main: avvio")
    if _istanza_gia_attiva():
        log.info("main: c'è già una copia in esecuzione, porto avanti quella")
        _porta_in_primo_piano_altra_istanza()
        return
    _identifica_app_a_windows()
    api = Api()
    try:
        parti_ridotto = bool(config.load_config().get("start_minimized"))
    except Exception:
        parti_ridotto = False
    # La pagina si apre come file://, non come http://127.0.0.1. Passando un
    # percorso, pywebview avvia un suo server HTTP locale e ci punta il WebView:
    # se il firewall di Windows non ha una regola per pythonw.exe (e per
    # impostazione predefinita non ce l'ha) quelle connessioni in loopback
    # vengono bloccate, la pagina non carica niente e la finestra resta bianca
    # e muta. Con file:// non serve nessun socket e non c'è niente da sbloccare.
    pagina = "file:///" + os.path.join(WEB_DIR, "index.html").replace("\\", "/")
    window = webview.create_window(
        # Titolo vuoto: la barra del titolo di Windows resta senza scritta (icona
        # e pulsanti restano), così "Calendar Hub" compare una volta sola, nella
        # nostra intestazione. L'identità nella barra applicazioni la dà comunque
        # l'icona + AppUserModelID impostato sopra.
        "",
        pagina,
        js_api=api,
        width=1180,
        height=820,
        min_size=(900, 600),
        # Senza cornice: niente barra del titolo di Windows, così l'intestazione
        # dell'app è l'unica in alto (come Teams). I pulsanti riduci/ingrandisci/
        # chiudi e il trascinamento li mette la UI; il ridimensionamento dai bordi
        # lo ricostruisce _aggancia_bordi in winforms.py (window procedure via
        # ctypes: l'override di WndProc da Python pythonnet non lo chiama).
        frameless=True,
        easy_drag=True,
        minimized=parti_ridotto,
    )
    api._window = window

    # Servizio promemoria in background.
    service = reminders.ReminderService()
    service.start()

    log.info("main: webview.start()")
    # L'icona vale anche per la barra delle applicazioni. La docstring di
    # pywebview la dà per sola GTK/QT, ma il backend WinForms la usa davvero.
    webview.start(icon=ICON_PATH if os.path.exists(ICON_PATH) else None)
    log.info("main: chiusura")
    service.stop()


if __name__ == "__main__":
    main()

