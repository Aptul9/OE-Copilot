#!/usr/bin/env python3
"""Prova della disponibilità pubblica: eventi finti, nessun server. La lancia la Action.

Verifica che escano solo orari e festività (mai titolo, luogo, azienda o altro dell'evento),
che i giorni interi e gli impegni senza durata non contino, che gli interruttori della barra
laterale non cambino il risultato né finiscano su disco, e che un guasto si dica.

Si esegue nell'immagine ui (ha le librerie e i moduli in /app):
    docker run --rm -v "$PWD/server/strumenti/prova_disponibilita.py:/prova.py:ro" oe-copilot-ui python /prova.py
oppure dal repository, con le librerie dell'app installate.
"""
import json
import os
import sys
import tempfile
from datetime import date, datetime

qui = os.path.dirname(os.path.abspath(__file__))
for cartella in ("/app", os.path.join(qui, "..", "..")):
    if os.path.exists(os.path.join(cartella, "app.py")):
        sys.path.insert(0, os.path.abspath(cartella))
        break
os.environ.setdefault("OE_DATI", tempfile.mkdtemp())

import app  # noqa: E402
import config  # noqa: E402
import disponibilita  # noqa: E402
import eventi  # noqa: E402

config.CONFIG_PATH = os.path.join(tempfile.mkdtemp(), "config.json")


def ev(inizio, fine, alias="ROAR", titolo="SEGRETO riunione col cliente", tutto_giorno=False, conto="mario@roar.example"):
    return {"id": "x|" + inizio, "title": titolo, "start": inizio, "end": fine, "allDay": tutto_giorno,
            "alias": alias, "account": conto, "location": "Via Segreta 1",
            "organizer": "Dott. Segreto", "online_url": "https://teams.example/segreto"}


G0 = date(2026, 10, 12)
FESTA = ev("2026-11-01T00:00:00", "2026-11-02T00:00:00", "Festività", "Ognissanti", True, config.FESTIVITA_NOME)
EVENTI = [
    ev("2026-10-12T09:00:00", "2026-10-12T10:00:00", "ROAR"),
    ev("2026-10-12T09:30:00", "2026-10-12T11:00:00", "ITV"),        # si sovrappone al primo
    ev("2026-10-12T09:30:00", "2026-10-12T11:00:00", "Prisma", "SEGRETO altro"),   # stesso orario
    ev("2026-10-12T11:00:00", "2026-10-12T12:00:00", "ITV"),        # tocca il secondo: stesso blocco
    ev("2026-10-12T14:00:00", "2026-10-12T15:00:00", "Prisma"),
    ev("2026-10-12T00:00:00", "2026-10-13T00:00:00", "ROAR", "SEGRETO ferie", True),   # giorno intero: non conta
    ev("2026-10-12T16:00:00", "2026-10-12T16:00:00", "ROAR"),       # senza durata
    ev("2026-10-12T22:00:00", "2026-10-13T01:30:00", "ROAR"),       # a cavallo della mezzanotte: com'è
    ev("boh", "2026-10-13T10:00:00", "ROAR"),                        # illeggibile: si salta
    ev("2026-10-05T09:00:00", "2026-10-05T10:00:00", "ROAR"),       # prima dell'intervallo
    ev("2026-10-20T09:00:00", "2026-10-20T10:00:00", "ROAR"),       # dopo l'intervallo
    ev("2026-10-11T23:00:00", "2026-10-12T00:30:00", "ROAR"),       # entra nell'intervallo
    ev("2026-10-18T23:00:00", "2026-10-19T01:00:00", "ROAR"),       # esce dall'intervallo (l'ultimo giorno è il 18)
    FESTA,
]

# --- disponibilita.impegni --------------------------------------------------
r = disponibilita.impegni(EVENTI, G0, 7)
atteso = [
    ["2026-10-11T23:00:00", "2026-10-12T00:30:00"],
    ["2026-10-12T09:00:00", "2026-10-12T12:00:00"],     # tre impegni sovrapposti o attaccati: un blocco
    ["2026-10-12T14:00:00", "2026-10-12T15:00:00"],
    ["2026-10-12T22:00:00", "2026-10-13T01:30:00"],
    ["2026-10-18T23:00:00", "2026-10-19T01:00:00"],
]
assert r == atteso, r
print("ok   solo gli orari, in ordine; sovrapposti e attaccati diventano un blocco solo")
print("ok   giorno intero, senza durata e illeggibili non contano; entra o esce dall'intervallo sì, fuori no")

# due che si toccano a cavallo della mezzanotte sono uno
unione = disponibilita.impegni([ev("2026-10-12T22:00:00", "2026-10-13T00:00:00"), ev("2026-10-13T00:00:00", "2026-10-13T01:00:00")], G0, 3)
assert unione == [["2026-10-12T22:00:00", "2026-10-13T01:00:00"]], unione
# un buco di un minuto resta un buco
assert len(disponibilita.impegni([ev("2026-10-12T09:00:00", "2026-10-12T10:00:00"), ev("2026-10-12T10:01:00", "2026-10-12T11:00:00")], G0, 1)) == 2
print("ok   a cavallo della mezzanotte uno solo; un buco vero resta un buco")

r = disponibilita.impegni(EVENTI, G0, 7, {"ITV"})
assert r == [["2026-10-12T09:30:00", "2026-10-12T12:00:00"]], r
assert disponibilita.impegni(EVENTI, G0, 7, set()) == []
print("ok   per gruppo: solo le aziende del gruppo, un gruppo vuoto non ha impegni")

aware = ev("2026-10-12T09:00:00+00:00", "2026-10-12T10:00:00+00:00")
locale = datetime.fromisoformat(aware["start"]).astimezone().replace(tzinfo=None).isoformat(timespec="seconds")
assert disponibilita.impegni([aware], date(2026, 10, 11), 3)[0][0] == locale
print("ok   orari con fuso: convertiti all'ora locale")

assert disponibilita.festivita(EVENTI, config.FESTIVITA_NOME) == [["2026-11-01T00:00:00", "2026-11-02T00:00:00", "Ognissanti"]]
assert disponibilita.festivita(EVENTI, "Altro") == []
print("ok   le festività sono quelle del calendario delle feste, non gli altri giorni interi")

for tutto in (disponibilita.impegni(EVENTI, G0, 7), disponibilita.impegni(EVENTI, G0, 7, {"ITV"})):
    testo = json.dumps(tutto)
    for segreto in ("SEGRETO", "ROAR", "ITV", "Prisma", "teams", "Segreta", "mario", "ferie"):
        assert segreto not in testo, segreto
print("ok   l'uscita ha solo date e ore: né titolo, né azienda, né luogo, né link")

# --- eventi.occupato e Api.get_disponibilita, con una lettura finta ----------
visto = {}


def lettura_finta(cfg, start_iso, end_iso, allow_cache=True):
    visto.update(cfg=cfg, start=start_iso, end=end_iso, allow_cache=allow_cache)
    return list(EVENTI), visto.get("errori", []), visto.get("meta", {"stale": False, "outlook_starting": False})


eventi.raccogli = lettura_finta
api = app.Api()
api.save_config({"disabled_calendars": ["remoto::a@roar.example", "ics_x"],
                 "gruppi_calendari": {"antonio": ["ITV"], "vuoto": []}})

out = api.get_disponibilita("2026-10-12", 7)
assert out["ok"] is True and out["completo"] is True and sorted(out) == ["completo", "festivita", "impegni", "ok"], out
assert out["impegni"] == atteso and out["festivita"] == [["2026-11-01T00:00:00", "2026-11-02T00:00:00", "Ognissanti"]]
print("ok   Api.get_disponibilita: impegni, festività, completo")
assert visto["start"] == "2026-10-12T00:00:00" and visto["end"] == "2026-10-19T00:00:00" and visto["allow_cache"] is False, visto
assert visto["cfg"]["disabled_calendars"] == [] and visto["cfg"]["_sola_lettura"] is True
assert config.load_config()["disabled_calendars"] == ["remoto::a@roar.example", "ics_x"]
print("ok   la lettura vede tutti i calendari, senza cache; gli interruttori veri restano")

# la copia di lavoro non scrive sul disco, nemmeno se una sorgente salva la configurazione
config.save_config(visto["cfg"])
assert config.load_config()["disabled_calendars"] == ["remoto::a@roar.example", "ics_x"]
assert "_sola_lettura" not in config.load_config()
config.save_config({**config.load_config(), "zoom_calendario": 77})
assert config.load_config()["zoom_calendario"] == 77
print("ok   save_config: la copia di lavoro non va su disco, la configurazione vera sì")

assert api.get_disponibilita("2026-10-12", 7, "antonio")["impegni"] == [["2026-10-12T09:30:00", "2026-10-12T12:00:00"]]
print("ok   per gruppo")
visto.clear()
assert api.get_disponibilita("2026-10-12", 7, "nessuno") == {"ok": False, "error": "Gruppo non trovato"} and not visto
print("ok   gruppo che non esiste: rifiutato prima di leggere i calendari")
for da, giorni in (("ieri", 7), ("2026-13-45", 7), ("2026-10-12", 0), ("2026-10-12", 63), ("2026-10-12", "x"), (None, 7)):
    assert api.get_disponibilita(da, giorni) == {"ok": False, "error": "Intervallo non valido"}, (da, giorni)
print("ok   data o numero di giorni non validi: rifiutati")

visto["errori"] = ["ROAR: Sul server l'aggiornamento è fermo da 20 minuti"]
out = api.get_disponibilita("2026-10-12", 1)
assert out["ok"] is True and out["completo"] is False and "ROAR" not in json.dumps(out), out
visto["errori"], visto["meta"] = [], {"stale": False, "outlook_starting": True}
assert api.get_disponibilita("2026-10-12", 1)["completo"] is False
visto["meta"] = {"stale": True, "outlook_starting": False}
assert api.get_disponibilita("2026-10-12", 1)["completo"] is False
print("ok   una sorgente in errore, ferma o con dati vecchi: completo falso, senza dire quale")


def lettura_rotta(*_a, **_k):
    raise RuntimeError("password=hunter2 in fondo a un errore")


eventi.raccogli = lettura_rotta
out = api.get_disponibilita("2026-10-12", 1)
assert out == {"ok": False, "error": "Calendari non leggibili"}, out
print("ok   un'eccezione non porta fuori il suo testo")
print("TUTTO OK")
