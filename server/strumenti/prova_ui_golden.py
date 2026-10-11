#!/usr/bin/env python3
"""Prova del servizio ui (app.Api) contro un agente finto: l'oracolo delle prove del gateway. La lancia la Action.

L'agente finto ripete le risposte registrate da prova_agente_golden.py (golden/agente.json), con lo stato del servizio e delle
caselle che ogni scenario chiede. Il servizio ui legge una cartella dei dati costruita da golden/ui/fixture.json (configurazione,
personalizzazione, eventi locali, copia degli eventi, un calendario ICS servito da un server finto).
- La lettura: risponde a una sequenza di chiamate (get_config, get_personalizzazione, get_calendars, get_events,
  get_disponibilita) con l'agente in ogni stato. Le risposte e le cartelle dei dati a fine sequenza si confrontano con
  golden/ui-lettura.json.
- La scrittura: una sequenza di modifiche (interruttori, gruppi, alias, calendari da link, eventi personali, configurazione); dopo
  ogni chiamata si registrano la risposta e lo stato (config.json, local_events.json, copie dei calendari ICS), in
  golden/ui-scritture.json.
Le stesse chiamate sulla stessa cartella le fa il gateway in TypeScript (server/web-ts/test/golden-lettura.test.ts e
golden-scritture.test.ts): finché il servizio ui esiste, qualunque differenza è un difetto del gateway.

Si esegue nell'immagine ui (ha le librerie e i moduli in /app):
    docker run --rm -v "$PWD/server/strumenti:/prove:ro" oe-copilot-ui python /prove/prova_ui_golden.py
Con --aggiorna riscrive i file registrati (montare /prove in scrittura), dopo aver letto la differenza.
"""
import copy
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

QUI = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.join(QUI, "golden")
REGISTRATO = os.path.join(GOLDEN, "ui-lettura.json")
REGISTRATO_SCRITTURE = os.path.join(GOLDEN, "ui-scritture.json")
CHIAVE = "chiave-di-prova"
# time.time() è fisso (e prima di qualunque file creato ora): la copia dei calendari ICS è sempre fresca, e l'ora di una copia degli
# eventi non dipende da quando gira la prova. L'ora che dice "Letto alle HH:MM" invece è quella vera, e si normalizza.
ORA_FISSA = 1_700_000_000.0


def leggi_json(*parti):
    with open(os.path.join(GOLDEN, *parti), encoding="utf-8") as f:
        return json.load(f)


def normalizza(dati):
    """Le ore del giorno cambiano a ogni lettura: si tolgono, il resto deve restare identico."""
    testo = json.dumps(dati, ensure_ascii=False, sort_keys=True)
    testo = re.sub(r"(Letto )?alle \d\d:\d\d", r"\1alle HH:MM", testo)
    testo = re.sub(r"http://127\.0\.0\.1:\d+", "http://127.0.0.1:PORTA", testo)   # la porta del server ICS di prova cambia a ogni giro
    testo = re.sub(r"ics_[0-9a-f]{12}", "ics_HASH", testo)   # e con lei l'id (sha1 del link) dei calendari da link aggiunti
    return json.loads(testo)


# ---- l'agente finto e il server dei calendari ICS -------------------------------------------------------------------------
def avvia_agente_finto(registrato, modi, stato):
    class Agente(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def _json(self, codice, dati):
            corpo = json.dumps(dati, ensure_ascii=False).encode("utf-8")
            self.send_response(codice)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(corpo)))
            self.end_headers()
            self.wfile.write(corpo)

        def do_GET(self):
            modo = modi[stato["modo"]]
            if "http" in modo:
                return self._json(modo["http"], {"errore": "finto"})
            if self.headers.get("Authorization") != "Bearer " + CHIAVE:
                return self._json(401, {"errore": "non autorizzato"})
            url = urlparse(self.path)
            q = parse_qs(url.query)
            if url.path == "/api/calendari":
                dati = copy.deepcopy(registrato["calendari"])
            elif url.path == "/api/eventi":
                chiave = "%s|%s" % (q["inizio"][0], q["fine"][0])
                if chiave not in registrato["eventi"]:
                    return self._json(404, {"errore": "finestra non registrata"})
                dati = copy.deepcopy(registrato["eventi"][chiave])
            else:
                return self._json(404, {"errore": "sconosciuto"})
            if "servizio" in modo:
                dati["servizio"] = modo["servizio"]
            for c in dati.get("caselle", []):
                c.update(modo.get("stati", {}).get(c["account"], {}))
            return self._json(200, dati)

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Agente)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return "http://127.0.0.1:%d" % httpd.server_address[1]


def avvia_server_ics():
    import functools
    import http.server

    class Silenzioso(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def guess_type(self, path):
            # come un server vero (Google dichiara il charset): senza, requests legge un testo text/* come ISO-8859-1
            return "text/calendar; charset=utf-8"

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Silenzioso, directory=os.path.join(GOLDEN, "ics")))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return "http://127.0.0.1:%d" % httpd.server_address[1]


def scrivi(percorso, dati):
    os.makedirs(os.path.dirname(percorso), exist_ok=True)
    with open(percorso, "w", encoding="utf-8", newline="\n") as f:
        json.dump(dati, f, ensure_ascii=False, indent=2)


def prepara_dati(fixture, profilo, ics_url):
    dati = tempfile.mkdtemp()
    config = json.loads(json.dumps(fixture["config"]).replace("{ICS}", ics_url))
    scrivi(os.path.join(dati, "config.json"), config)
    scrivi(os.path.join(dati, "personalizzazione.json"), fixture["profili"][profilo]["personalizzazione"])
    scrivi(os.path.join(dati, "local_events.json"), fixture["locali"])
    scrivi(os.path.join(dati, "agente_caselle.json"), fixture["caselle_note"])
    scrivi(os.path.join(dati, "events_cache.json"), fixture["cache_eventi"])
    os.makedirs(os.path.join(dati, "ics_cache"))
    shutil.copy(os.path.join(GOLDEN, "ics", "festivita.ics"), os.path.join(dati, "ics_cache", "builtin__festivita-it.ics"))
    return dati


def stato_finale(dati):
    def leggi(nome):
        with open(os.path.join(dati, nome), encoding="utf-8") as f:
            return json.load(f)
    cache = leggi("events_cache.json")
    return normalizza({
        "config.json": leggi("config.json"),
        "agente_caselle.json": leggi("agente_caselle.json"),
        "local_events.json": leggi("local_events.json"),
        "events_cache.json": {k: {"stamp": v.get("stamp"), "eventi": len(v.get("eventi", []))} for k, v in cache.items()},
        "ics_cache": sorted(os.listdir(os.path.join(dati, "ics_cache"))),
    })


# ---- un profilo: un processo (config.py legge la personalizzazione una volta, all'avvio) ----------------------------------------
def esegui_profilo(profilo, uscita):
    registrato = leggi_json("agente.json")
    fixture = leggi_json("ui", "fixture.json")
    stato = {"modo": "normale"}
    agente = avvia_agente_finto(registrato, fixture["modi"], stato)
    dati = prepara_dati(fixture, profilo, avvia_server_ics())
    os.environ.update(OE_DATI=dati, OE_AGENTE_URL=agente, OE_AGENTE_CHIAVE=CHIAVE)
    time.time = lambda: ORA_FISSA
    for cartella in ("/app", os.path.join(QUI, "..", "..")):
        if os.path.exists(os.path.join(cartella, "app.py")):
            sys.path.insert(0, os.path.abspath(cartella))
            break
    import logging
    logging.disable(logging.CRITICAL)
    import app
    api = app.Api()
    risultati = {}
    for s in fixture["scenari"]:
        if s["profilo"] != profilo:
            continue
        stato["modo"] = s["modo"]
        try:
            risultati[s["nome"]] = normalizza(getattr(api, s["metodo"])(*s["args"]))
        except Exception as e:
            risultati[s["nome"]] = {"eccezione": e.__class__.__name__}
    scrivi(uscita, {"risposte": risultati, "dati": stato_finale(dati)})


# ---- le scritture: un processo, una cartella dei dati, la sequenza in ordine --------------------------------------------------
def stato_scritture(dati):
    """Quello che le scritture cambiano: la configurazione, gli eventi personali, le copie dei calendari da link."""
    def leggi(nome):
        with open(os.path.join(dati, nome), encoding="utf-8") as f:
            return json.load(f)
    return normalizza({
        "config.json": leggi("config.json"),
        "local_events.json": leggi("local_events.json"),
        "ics_cache": sorted(os.listdir(os.path.join(dati, "ics_cache"))),
    })


def esegui_scritture(uscita):
    registrato = leggi_json("agente.json")
    fixture = leggi_json("ui", "fixture.json")
    stato = {"modo": "normale"}
    agente = avvia_agente_finto(registrato, fixture["modi"], stato)
    ics_url = avvia_server_ics()
    dati = prepara_dati(fixture, "base", ics_url)
    os.environ.update(OE_DATI=dati, OE_AGENTE_URL=agente, OE_AGENTE_CHIAVE=CHIAVE)
    time.time = lambda: ORA_FISSA
    for cartella in ("/app", os.path.join(QUI, "..", "..")):
        if os.path.exists(os.path.join(cartella, "app.py")):
            sys.path.insert(0, os.path.abspath(cartella))
            break
    import logging
    logging.disable(logging.CRITICAL)
    import app
    api = app.Api()
    passi = {}
    ultimo_feed = ""
    for s in fixture["scritture"]:
        args = json.loads(json.dumps(s["args"]).replace("{ICS}", ics_url).replace("{ULTIMO_FEED}", ultimo_feed))
        try:
            risposta = getattr(api, s["metodo"])(*args)
        except Exception as e:
            risposta = {"eccezione": e.__class__.__name__}
        if s["metodo"] == "add_ics_calendar" and isinstance(risposta, dict) and risposta.get("ok"):
            ultimo_feed = risposta["feed"]["id"]
        passi[s["nome"]] = {"risposta": normalizza(risposta), "stato": stato_scritture(dati)}
    scrivi(uscita, passi)


def differenza(registrato, ottenuto):
    righe = []
    for profilo in sorted(set(registrato) | set(ottenuto)):
        a, b = registrato.get(profilo, {}), ottenuto.get(profilo, {})
        for nome in sorted(set(a.get("risposte", {})) | set(b.get("risposte", {}))):
            if a.get("risposte", {}).get(nome) != b.get("risposte", {}).get(nome):
                righe.append("%s / %s: diverso" % (profilo, nome))
        for nome in sorted(set(a.get("dati", {})) | set(b.get("dati", {}))):
            if a.get("dati", {}).get(nome) != b.get("dati", {}).get(nome):
                righe.append("%s / dati a fine sequenza, %s: diverso" % (profilo, nome))
    return "\n".join(righe[:40])


def differenza_scritture(registrato, ottenuto):
    righe = []
    for nome in sorted(set(registrato) | set(ottenuto)):
        a, b = registrato.get(nome, {}), ottenuto.get(nome, {})
        if a.get("risposta") != b.get("risposta"):
            righe.append("%s: risposta diversa" % nome)
        for parte in sorted(set(a.get("stato", {})) | set(b.get("stato", {}))):
            if a.get("stato", {}).get(parte) != b.get("stato", {}).get(parte):
                righe.append("%s: stato diverso, %s" % (nome, parte))
    return "\n".join(righe[:40])


def in_un_processo(argomenti):
    """Esegue una parte della sequenza in un processo suo (config.py legge la personalizzazione una volta, all'avvio). -> il JSON scritto."""
    tmp = os.path.join(tempfile.mkdtemp(), "uscita.json")
    r = subprocess.run([sys.executable, os.path.abspath(__file__), *argomenti, "--uscita", tmp])
    if r.returncode:
        return None
    with open(tmp, encoding="utf-8") as f:
        return json.load(f)


def main():
    if "--profilo" in sys.argv:
        esegui_profilo(sys.argv[sys.argv.index("--profilo") + 1], sys.argv[sys.argv.index("--uscita") + 1])
        return 0
    if "--scritture" in sys.argv:
        esegui_scritture(sys.argv[sys.argv.index("--uscita") + 1])
        return 0
    fixture = leggi_json("ui", "fixture.json")
    ottenuto = {}
    for profilo in fixture["profili"]:
        ottenuto[profilo] = in_un_processo(["--profilo", profilo])
        if ottenuto[profilo] is None:
            print("KO   il profilo %s non è finito" % profilo)
            return 1
        print("ok   profilo %s: %d chiamate" % (profilo, len(ottenuto[profilo]["risposte"])))
    scritto = in_un_processo(["--scritture"])
    if scritto is None:
        print("KO   la sequenza delle scritture non è finita")
        return 1
    print("ok   scritture: %d chiamate" % len(scritto))
    if "--aggiorna" in sys.argv:
        scrivi(REGISTRATO, ottenuto)
        scrivi(REGISTRATO_SCRITTURE, scritto)
        print("riscritti %s e %s" % (REGISTRATO, REGISTRATO_SCRITTURE))
        return 0
    esito = 0
    for percorso, nuovo, confronto, cosa in ((REGISTRATO, ottenuto, differenza, "la lettura"), (REGISTRATO_SCRITTURE, scritto, differenza_scritture, "le scritture")):
        with open(percorso, encoding="utf-8") as f:
            registrato = json.load(f)
        if registrato == nuovo:
            print("ok   %s del servizio ui: come registrato in golden/%s" % (cosa, os.path.basename(percorso)))
        else:
            print("KO   %s del servizio ui: diverso da golden/%s\n%s" % (cosa, os.path.basename(percorso), confronto(registrato, nuovo)))
            esito = 1
    if not esito:
        print("TUTTO OK")
    return esito


if __name__ == "__main__":
    sys.exit(main())
