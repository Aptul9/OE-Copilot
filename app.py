"""OE Copilot - la logica dell'app: calendari di più account, raccolti e uniti."""
import logging
import os
import time
from datetime import date

import caselle
import config
import eventi as raccolta_eventi
import ics_feed
import local_events

# Cartella della pagina: ci stanno i loghi (web/logos) che il servizio ui serve.
WEB_DIR = os.path.join(config.RESOURCE_DIR, "web")

# requests/urllib3 in DEBUG riempiono il log di righe di connessione e
# seppelliscono le nostre: qui interessano solo i loro problemi veri.
logging.getLogger("urllib3").setLevel(logging.WARNING)
log = logging.getLogger("calendarhub")


class Api:
    """Metodi che il servizio ui (server/ui/server.py) espone al gateway: POST /api/<metodo>."""

    # ---- Configurazione ------------------------------------------------
    def get_config(self):
        return config.load_config()

    def save_config(self, new_cfg):
        cfg = config.load_config()
        cfg.update(new_cfg or {})
        config.save_config(cfg)
        return {"ok": True}

    # ---- Calendari -----------------------------------------------------
    def get_calendars(self, force=False):
        """Calendari delle caselle + quelli sottoscritti da link ICS."""
        t0 = time.time()
        log.info("get_calendars: inizio (force=%s)", force)
        cfg = config.load_config()
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
        # Vanno considerate TUTTE le sorgenti, non solo le caselle e i feed ICS:
        # altrimenti le voci che arrivano dal calendario locale restano accese
        # qualunque cosa si clicchi.
        try:
            caselle_lette = raccolta_eventi.calendari_caselle(cfg)
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE:
            caselle_lette = []   # senza le caselle si spengono comunque le altre voci
        tutti = caselle_lette + raccolta_eventi.calendari_extra(cfg)
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
            caselle_lette = raccolta_eventi.calendari_caselle(cfg)
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE:
            caselle_lette = []
        tutti = caselle_lette + raccolta_eventi.calendari_extra(cfg)
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
            caselle_lette = raccolta_eventi.calendari_caselle(cfg)
        except raccolta_eventi.SORGENTE_NON_DISPONIBILE:
            caselle_lette = []
        tutti = caselle_lette + raccolta_eventi.calendari_extra(cfg)
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

    def get_disponibilita(self, da, giorni=7, gruppo=""):
        """Cosa può sapere la pagina pubblica dei `giorni` giorni da `da` (AAAA-MM-GG).

        La legge il gateway per la pagina senza accesso, che è il calendario di sempre con
        gli impegni ridotti a "Occupato": escono solo gli orari ({"impegni": [[inizio, fine]]}),
        le festività e `completo`, falso se una sorgente non ha risposto bene. Gli errori
        restano qui nel log.
        """
        try:
            primo = date.fromisoformat(str(da))
            giorni = int(giorni)
            if not 1 <= giorni <= 62:
                raise ValueError
        except (ValueError, TypeError):
            return {"ok": False, "error": "Intervallo non valido"}
        try:
            impegni, festivita, completo = raccolta_eventi.occupato(
                config.load_config(), primo, giorni, str(gruppo or ""))
        except ValueError as e:
            return {"ok": False, "error": str(e)}
        except Exception:
            log.exception("get_disponibilita: eccezione")
            return {"ok": False, "error": "Calendari non leggibili"}
        return {"ok": True, "impegni": impegni, "festivita": festivita, "completo": completo}

    def get_personalizzazione(self):
        """Impostazioni dell'interfaccia di chi usa l'app (sigle, loghi, schede).

        Arrivano da personalizzazione.json: nel codice non ci sono nomi di
        aziende. Con il file assente l'interfaccia parte con i valori neutri.
        """
        return {"interfaccia": config.INTERFACCIA}
