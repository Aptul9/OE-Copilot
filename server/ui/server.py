"""OE Copilot: la logica dell'app, servita al gateway.

La logica è la classe app.Api, copiata qui da strumenti/prepara_ui.sh con i moduli che usa.
Questo file è solo il server HTTP: ogni metodo di Api in POST /api/<metodo> con gli
argomenti in una lista JSON, e i loghi delle aziende in GET /logos/<file>. La pagina la
serve il gateway. Non ha password: ci arriva solo il gateway, che la chiede (vedi
docker-compose.yml).
"""
import json
import logging
import mimetypes
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlsplit

logging.basicConfig(level=logging.INFO,
                    format="%(asctime)s [%(threadName)s] %(name)s: %(message)s")

import app  # noqa: E402

log = logging.getLogger("ui")

# Quello che la pagina può chiamare: tutti i metodi di Api.
METODI = {
    "get_config", "save_config", "get_personalizzazione",
    "get_calendars", "get_events", "get_disponibilita",
    "set_calendar_enabled", "set_account_enabled", "set_azienda_enabled",
    "mostra_solo_azienda", "mostra_solo_gruppo", "set_account_alias",
    "add_ics_calendar", "remove_ics_calendar", "add_local_event", "delete_local_event",
}
MAX_CORPO = 1_000_000

API = app.Api()
WEB = os.path.realpath(app.WEB_DIR)
# I loghi delle aziende stanno solo sul server (dati di chi usa l'app, fuori dal repository): /logos/<file> si cerca prima qui,
# poi tra quelli della pagina (web/logos). Si dichiarano in personalizzazione.json, interfaccia.loghi.
LOGHI = os.path.realpath(os.path.join(app.config.DATA_DIR, "logos"))


class Gestore(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "OECopilot"

    def log_message(self, formato, *args):
        if self.path != "/salute":     # il controllo di Docker, ogni 30 secondi
            log.info("%s %s", self.address_string(), formato % args)

    def _rispondi(self, codice, corpo=b"", tipo="application/json; charset=utf-8"):
        self.send_response(codice)
        self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(corpo)))
        self.send_header("Cache-Control", "no-cache")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(corpo)

    def _json(self, codice, dati):
        self._rispondi(codice, json.dumps(dati, ensure_ascii=False).encode("utf-8"))

    def do_GET(self):
        percorso = unquote(urlsplit(self.path).path)
        if percorso == "/salute":
            return self._rispondi(200, b"ok", "text/plain")
        # Solo i loghi: prima quelli del server (cartella dati), poi quello che sta nella pagina
        # dell'app (web/logos). Mai fuori da queste due cartelle.
        nome = percorso[len("/logos/"):] if percorso.startswith("/logos/") else None
        file = None
        for radice in (LOGHI, os.path.join(WEB, "logos")) if nome is not None else ():
            radice = os.path.realpath(radice)
            prova = os.path.realpath(os.path.join(radice, nome))
            if os.path.commonpath([radice, prova]) == radice and os.path.isfile(prova):
                file = prova
                break
        if file is None:
            return self._json(404, {"error": "non trovato"})
        tipo = mimetypes.guess_type(file)[0] or "application/octet-stream"
        with open(file, "rb") as f:
            self._rispondi(200, f.read(), tipo)

    do_HEAD = do_GET

    def do_POST(self):
        try:
            lunghezza = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            lunghezza = -1
        if not 0 <= lunghezza <= MAX_CORPO:
            self.close_connection = True    # corpo non letto: la connessione non è più allineata
            return self._json(413 if lunghezza > MAX_CORPO else 400,
                              {"error": "lunghezza della richiesta non valida"})
        # Il corpo si legge sempre per intero, anche se poi la richiesta si rifiuta:
        # se resta lì, sulla stessa connessione verrebbe letto come la richiesta dopo.
        corpo = self.rfile.read(lunghezza)
        parti = urlsplit(self.path).path.split("/")
        if len(parti) != 3 or parti[1] != "api" or parti[2] not in METODI:
            return self._json(404, {"error": "metodo non disponibile"})
        # Il browser aggiunge da solo la password a ogni richiesta verso questo sito,
        # anche se parte da un'altra pagina. Con application/json una pagina altrui
        # non può nemmeno mandarla (serve un permesso che qui non si dà).
        if self.headers.get_content_type() != "application/json":
            return self._json(415, {"error": "serve application/json"})
        if self.headers.get("Sec-Fetch-Site", "same-origin") not in ("same-origin", "none"):
            return self._json(403, {"error": "richiesta partita da un altro sito"})
        try:
            args = json.loads(corpo or b"[]")
            if not isinstance(args, list):
                raise ValueError("gli argomenti vanno in una lista")
        except ValueError as e:
            return self._json(400, {"error": str(e)})
        try:
            risposta = json.dumps(getattr(API, parti[2])(*args), ensure_ascii=False).encode("utf-8")
        except Exception as e:
            log.exception("api.%s", parti[2])
            return self._json(500, {"error": str(e) or e.__class__.__name__})
        self._rispondi(200, risposta)


if __name__ == "__main__":
    # In Docker serve 0.0.0.0 (ci arriva Caddy dalla rete di compose); per una prova sul
    # PC si mette OE_ASCOLTO=127.0.0.1, perché qui non c'è nessuna password.
    ascolto = os.environ.get("OE_ASCOLTO") or "0.0.0.0"
    porta = int(os.environ.get("OE_PORTA") or 8181)
    log.info("interfaccia web su %s:%d, dati in %s, agente %s", ascolto, porta,
             app.config.DATA_DIR, app.raccolta_eventi.agente_remoto.URL_DIRETTO or "(tunnel)")
    ThreadingHTTPServer((ascolto, porta), Gestore).serve_forever()
