#!/usr/bin/env python3
"""Prova dei gruppi di calendari: calendari finti, nessun server. La lancia la Action.

Si esegue nell'immagine ui (ha le librerie e i moduli in /app):
    docker run --rm -v "$PWD/server/strumenti/prova_gruppi.py:/prova.py:ro" oe-copilot-ui python /prova.py
oppure dal repository, con le librerie dell'app installate.
"""
import os
import sys
import tempfile
import types

# app.py importa la finestra, i promemoria e Windows: qui non servono (come in server/ui/server.py)
for nome in ("webview", "reminders", "teams_link", "outlook_com"):
    sys.modules[nome] = types.ModuleType(nome)

qui = os.path.dirname(os.path.abspath(__file__))
for cartella in ("/app", os.path.join(qui, "..", "..")):
    if os.path.exists(os.path.join(cartella, "app.py")):
        sys.path.insert(0, os.path.abspath(cartella))
        break
os.environ.setdefault("OE_DATI", tempfile.mkdtemp())

import app  # noqa: E402
import config  # noqa: E402

config.CONFIG_PATH = os.path.join(tempfile.mkdtemp(), "config.json")

# --- config.membri_gruppo ---------------------------------------------------
assert config.membri_gruppo({}, "a") is None
assert config.membri_gruppo({"gruppi_calendari": []}, "a") is None
assert config.membri_gruppo({"gruppi_calendari": {"a": "ITV"}}, "a") is None
assert config.membri_gruppo({"gruppi_calendari": {"a": ["ITV", 3]}}, "b") is None
assert config.membri_gruppo({"gruppi_calendari": {"a": ["ITV", 3]}}, "a") == {"ITV", "3"}
assert config.membri_gruppo({"gruppi_calendari": {"a": []}}, "a") == set()
print("ok   membri_gruppo: assente, storto, vuoto, valido")

# --- config.rinomina_nei_gruppi ---------------------------------------------
cfg = {"gruppi_calendari": {"a": ["ITV", "ROAR"], "b": ["ROAR"], "c": ["ITV", "ITV Group"]}}
config.rinomina_nei_gruppi(cfg, "ITV", "ITV Group")
assert cfg["gruppi_calendari"] == {"a": ["ITV Group", "ROAR"], "b": ["ROAR"], "c": ["ITV Group"]}, cfg
config.rinomina_nei_gruppi(cfg, "ITV Group", "ITV Group")
config.rinomina_nei_gruppi({"gruppi_calendari": "x"}, "a", "b")
config.rinomina_nei_gruppi({}, "a", "b")
print("ok   rinomina_nei_gruppi: stesso ordine, nessun doppione, dati storti ignorati")

# --- Api.mostra_solo_gruppo e mostra_solo_azienda ---------------------------
CASELLE = [
    {"id": "remoto::a@itv.com", "account": "a@itv.com", "alias": "ITV", "enabled": True},
    {"id": "remoto::b@roar.com", "account": "b@roar.com", "alias": "ROAR", "enabled": True},
    {"id": "remoto::c@roar.com", "account": "c@roar.com", "alias": "ROAR", "enabled": True},
]
EXTRA = [{"id": "ics_x", "account": "Festività", "enabled": True}]
app.raccolta_eventi.calendari_caselle = lambda cfg, force=False: CASELLE
app.raccolta_eventi.calendari_extra = lambda cfg: EXTRA

api = app.Api()
spenti = lambda: config.load_config()["disabled_calendars"]  # noqa: E731

api.save_config({"gruppi_calendari": {"antonio": ["ITV"], "samuele": ["ROAR", "ITV"], "vuoto": []}})

assert api.mostra_solo_gruppo("nessuno") == {"ok": False, "error": "Gruppo non trovato"}
assert spenti() == [], spenti()
print("ok   gruppo che non esiste: rifiutato, niente cambia")

assert api.mostra_solo_gruppo("antonio") == {"ok": True}
assert spenti() == ["ics_x", "remoto::b@roar.com", "remoto::c@roar.com"], spenti()
print("ok   gruppo antonio: resta accesa ITV, spente ROAR (due calendari) e le festività")

assert api.mostra_solo_gruppo("samuele") == {"ok": True}
assert spenti() == ["ics_x"], spenti()
print("ok   un'azienda in due gruppi si accende con l'uno e con l'altro")

assert api.mostra_solo_azienda("ROAR") == {"ok": True}
assert spenti() == ["ics_x", "remoto::a@itv.com"], spenti()
assert api.mostra_solo_azienda("") == {"ok": True}
assert spenti() == [], spenti()
print("ok   mostra_solo_azienda come prima (una sola, poi tutte)")

assert api.mostra_solo_gruppo("antonio") == {"ok": True}
assert config.load_config()["gruppi_calendari"] == {"antonio": ["ITV"], "samuele": ["ROAR", "ITV"], "vuoto": []}
print("ok   i gruppi restano com'erano dopo il filtro")

# --- rinominare un'azienda non la toglie dai gruppi ---------------------------
api.set_account_alias("a@itv.com", "ITV")             # nome automatico "Itv" -> "ITV": nessun gruppo lo nomina
assert config.load_config()["gruppi_calendari"]["antonio"] == ["ITV"]
api.set_account_alias("a@itv.com", "ITV Group")
assert config.load_config()["gruppi_calendari"] == {
    "antonio": ["ITV Group"], "samuele": ["ROAR", "ITV Group"], "vuoto": []}, config.load_config()["gruppi_calendari"]
print("ok   rinomina: l'azienda cambia nome anche nei gruppi")
api.set_account_alias("a@itv.com", "")                # torna al nome automatico
assert config.load_config()["gruppi_calendari"]["antonio"] == [config.alias_per_account({}, ["a@itv.com"])["a@itv.com"]]
print("ok   nome automatico: il gruppo lo segue")
print("TUTTO OK")
