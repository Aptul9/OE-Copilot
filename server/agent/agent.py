"""Agente di Oe Copilot sulla VM: legge i calendari e li serve al gateway.

L'unica fonte e' il calendario di sistema del telefono Android virtuale, tenuto
aggiornato da client veri con i login fatti dentro di loro: Outlook per le
caselle Microsoft, i servizi Google del telefono per i calendari Google. Il servizio
oe-copilot-sincronizza della VM li tiene aggiornati ogni 30 secondi e lascia un
segno di vita (battito.json) che l'agente riporta: cosi' l'app sa se quello che
legge e' fresco o se il meccanismo si e' fermato.

API (tutte con "Authorization: Bearer <API_KEY>"):
  GET /api/eventi?inizio=ISO&fine=ISO  eventi del periodo + stato di ogni casella
  GET /api/calendari                   le caselle, con stato e numero di appuntamenti
  GET /api/salute                      stato delle caselle, senza rileggere
  POST /api/ics                        {"testo": ICS[, "inizio": ISO, "fine": ISO]}: il nome del calendario, o i suoi
                                       eventi nel periodo con le ricorrenze espanse (ics.py, nessun accesso alla rete)
"""
import hmac
import json
import logging
import os
import threading
import time
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import ics
from fonte_android import CalendarioAndroid

logging.basicConfig(level=logging.INFO,
                    format="%(asctime)s %(levelname)s %(name)s: %(message)s")
log = logging.getLogger("agente")

API_KEY = os.environ.get("API_KEY", "")
PORTA = int(os.environ.get("PORTA", "8180"))
CACHE_S = 15   # letture ravvicinate dello stesso periodo: una sola (il telefono si aggiorna ogni 30 s)
MAX_CORPO = 8_000_000   # il testo di un calendario ICS da espandere (POST /api/ics)
# Le fonti non stanno nel codice: fonti.json (solo sulla VM) elenca le caselle
# da leggere dal calendario del telefono (tipo "android").
FONTI_PATH = os.environ.get("FONTI", "/app/fonti.json")
ANDROID_DB = os.environ.get("ANDROID_DB", "/android-calendario")
# Segno di vita scritto a ogni giro dal servizio che aggiorna il telefono.
BATTITO = os.environ.get("BATTITO", "/stato/battito.json")


def _carica_fonti():
    with open(FONTI_PATH, "r", encoding="utf-8") as f:
        voci = json.load(f)
    fonti = []
    for v in voci:
        if v.get("tipo") == "android":
            fonti.append(CalendarioAndroid(v["id"], v["nome"], v["account"], ANDROID_DB,
                                           v.get("calendari")))
        else:
            log.error("fonte sconosciuta, ignorata: %r", v)
    return fonti


FONTI = _carica_fonti()

_cache = {}
_cache_lock = threading.Lock()


def _locale(testo):
    """ISO con o senza fuso -> datetime locale senza fuso."""
    d = datetime.fromisoformat(testo.replace("Z", "+00:00"))
    if d.tzinfo is not None:
        d = d.astimezone().replace(tzinfo=None)
    return d


def _servizio():
    """Segno di vita del servizio che aggiorna il telefono, o None se manca.

    "eta_s" dice da quanti secondi il servizio non passa: se cresce, i calendari
    del telefono non vengono piu' rinfrescati e l'app deve dirlo, invece di
    mostrare per buoni dati fermi.
    """
    try:
        with open(BATTITO, "r", encoding="utf-8") as f:
            b = json.load(f)
        return {"eta_s": max(0, int(time.time() - float(b["quando"]))),
                "pausa": bool(b.get("pausa")), "telefono": bool(b.get("telefono", True))}
    except (OSError, ValueError, KeyError, TypeError):
        return None


def raccogli(inizio, fine):
    chiave = (inizio.isoformat(), fine.isoformat())
    with _cache_lock:
        voce = _cache.get(chiave)
        if voce and time.time() - voce[0] < CACHE_S:
            return voce[1]
    eventi, errori = [], []
    for fonte in FONTI:
        try:
            trovati, err = fonte.eventi(inizio, fine)
        except Exception as e:
            # Una fonte rotta non deve portarsi via le altre.
            log.exception("fonte %s fallita", fonte.nome)
            trovati, err = [], ["%s: %s" % (fonte.nome, e)]
        eventi += trovati
        errori += err
    risposta = {"eventi": eventi, "errori": errori,
                "fonti": {f.nome: f.stato for f in FONTI},
                "caselle": [f.descrizione() for f in FONTI],
                "servizio": _servizio(),
                "generato": datetime.now().isoformat(timespec="seconds")}
    with _cache_lock:
        _cache[chiave] = (time.time(), risposta)
        # La cache non deve crescere all'infinito: si tengono le 50 piu' recenti.
        if len(_cache) > 50:
            for k, _ in sorted(_cache.items(), key=lambda kv: kv[1][0])[:-50]:
                _cache.pop(k, None)
    return risposta


class Gestore(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        log.info("%s %s", self.address_string(), fmt % args)

    def _json(self, codice, dati):
        corpo = json.dumps(dati, ensure_ascii=False).encode("utf-8")
        self.send_response(codice)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(corpo)))
        self.end_headers()
        self.wfile.write(corpo)

    def _autorizzato(self):
        atteso = "Bearer " + API_KEY
        return bool(API_KEY) and hmac.compare_digest(self.headers.get("Authorization", ""), atteso)

    def do_POST(self):
        if not self._autorizzato():
            return self._json(401, {"errore": "non autorizzato"})
        if urlparse(self.path).path != "/api/ics":
            return self._json(404, {"errore": "sconosciuto"})
        try:
            lunghezza = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            lunghezza = -1
        if not 0 < lunghezza <= MAX_CORPO:
            self.close_connection = True     # corpo non letto: la connessione non è più allineata
            return self._json(413 if lunghezza > MAX_CORPO else 400, {"errore": "lunghezza della richiesta non valida"})
        try:
            richiesta = json.loads(self.rfile.read(lunghezza))
            testo = richiesta["testo"]
            periodo = (richiesta.get("inizio"), richiesta.get("fine"))
            if not isinstance(testo, str) or (None in periodo and periodo != (None, None)):
                raise ValueError
            intervallo = None if periodo[0] is None else (_locale(periodo[0]), _locale(periodo[1]))
        except (ValueError, KeyError, TypeError, AttributeError):
            return self._json(400, {"errore": "serve {\"testo\": ..., \"inizio\": ..., \"fine\": ...}"})
        try:
            if intervallo is None:
                return self._json(200, {"ok": True, "nome": ics.valida(testo)})
            return self._json(200, {"ok": True, "eventi": ics.eventi(testo, *intervallo)})
        except Exception as e:
            # un testo che non è un calendario non deve fermare l'agente: lo si dice a chi l'ha mandato
            return self._json(200, {"ok": False, "errore": str(e), "valore": isinstance(e, (ValueError, TypeError))})

    def do_GET(self):
        if not self._autorizzato():
            return self._json(401, {"errore": "non autorizzato"})
        url = urlparse(self.path)
        q = parse_qs(url.query)
        if url.path == "/api/salute":
            return self._json(200, {"fonti": {f.nome: f.stato for f in FONTI},
                                    "servizio": _servizio()})
        if url.path == "/api/calendari":
            caselle = []
            for f in FONTI:
                try:
                    caselle.append(f.riepilogo())
                except Exception as e:
                    log.exception("fonte %s: riepilogo fallito", f.nome)
                    caselle.append({"id": f.id, "account": f.account, "nome": f.nome,
                                    "tipo": f.tipo, "eventi": None,
                                    "stato": "errore", "msg": str(e)})
            return self._json(200, {"caselle": caselle, "servizio": _servizio()})
        if url.path == "/api/eventi":
            try:
                inizio = _locale(q["inizio"][0])
                fine = _locale(q["fine"][0])
            except (KeyError, IndexError, ValueError) as e:
                return self._json(400, {"errore": "periodo non valido (%s)" % e})
            return self._json(200, raccogli(inizio, fine))
        return self._json(404, {"errore": "sconosciuto"})


if __name__ == "__main__":
    if not API_KEY:
        log.error("API_KEY mancante: l'API resterebbe aperta a chiunque, non parto")
        raise SystemExit(2)
    log.info("agente in ascolto sulla porta %d, fonti: %s", PORTA, [f.nome for f in FONTI])
    ThreadingHTTPServer(("0.0.0.0", PORTA), Gestore).serve_forever()
