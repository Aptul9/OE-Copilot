#!/usr/bin/env python3
"""oracolo.py <rileva_accessi.py> <cartella dei casi> <uscita>

Fa girare il main() di rileva_accessi.py (il file di server/strumenti, non modificato) su ogni caso: l'ora (time.time) e le risposte di
`docker exec <telefono> dumpsys content` e `date` vengono dai file del caso, il resto è il codice vero (file, sqlite3, base64, regex).
Per ogni caso scrive in <uscita>/<caso>: stato/ (copia di quello del caso, dove il rilevatore scrive accessi.json e google-sync.json),
stdout.txt, stderr.txt, comandi.txt (i comandi chiesti al telefono, con il timeout) e crash.txt se main() si ferma con un errore.
"""
import contextlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import time

sorgente, casi, uscita = sys.argv[1:4]
ambiente = dict(os.environ)
corsa_vera = subprocess.run
tempo_vero = time.time


def esegui(nome):
    caso = os.path.join(casi, nome)
    fuori = os.path.join(uscita, nome)
    adesso = float(open(os.path.join(caso, "adesso")).read())
    os.environ.clear()
    os.environ.update(ambiente)
    os.environ.update(json.load(open(os.path.join(caso, "env.json"))))
    shutil.rmtree(fuori, ignore_errors=True)
    shutil.copytree(os.path.join(caso, "stato"), os.path.join(fuori, "stato"))
    sys.argv = ["rileva_accessi.py", os.path.join(caso, "dati"), os.path.join(fuori, "stato", "accessi.json")]
    comandi = []

    def finto_run(args, **kw):
        comandi.append("%s timeout=%s" % (" ".join(args[3:]), kw.get("timeout")))
        if args[:3] != ["docker", "exec", os.environ.get("TELEFONO", "oe-copilot-telefono")] or not kw.get("text") or not kw.get("capture_output"):
            raise AssertionError("comando non previsto: %r %r" % (args, kw))
        file = {"dumpsys content": "dumpsys-content.txt", "date +%Y-%m-%d %H:%M:%S": "date.txt"}.get(" ".join(args[3:]))
        if file is None:
            raise AssertionError("comando non previsto: %r" % (args,))
        percorso = os.path.join(caso, file)
        if not os.path.exists(percorso):
            return subprocess.CompletedProcess(args, 1, stdout="", stderr="")
        return subprocess.CompletedProcess(args, 0, stdout=open(percorso, "rb").read().decode("utf-8"), stderr="")

    subprocess.run = finto_run
    time.time = lambda: adesso
    spec = importlib.util.spec_from_file_location("rileva_accessi", sorgente)
    modulo = importlib.util.module_from_spec(spec)
    out, err = io.StringIO(), io.StringIO()
    crash = None
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            spec.loader.exec_module(modulo)
            modulo.main()
    except Exception as e:  # un errore che main() non intercetta
        crash = type(e).__name__
    finally:
        subprocess.run = corsa_vera
        time.time = tempo_vero
    for file, testo in (("stdout.txt", out.getvalue()), ("stderr.txt", err.getvalue()), ("comandi.txt", "\n".join(comandi) + "\n")):
        open(os.path.join(fuori, file), "w", encoding="utf-8", newline="").write(testo)
    if crash:
        open(os.path.join(fuori, "crash.txt"), "w", encoding="utf-8").write(crash + "\n")


nomi = sorted(n for n in os.listdir(casi) if os.path.isdir(os.path.join(casi, n)))
for nome in nomi:
    esegui(nome)
print("oracolo: %d casi" % len(nomi))
