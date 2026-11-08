#!/usr/bin/env python3
"""Controlla che nessun dato di personalizzazione.json sia finito nei file del repository.

Indirizzi, domini e nomi di personalizzazione.json non devono stare nel codice: il codice va condiviso, i dati di
chi usa l'app stanno solo nella sua personalizzazione (esclusa dal repository, vedi .gitignore). Il controllo li cerca
uno per uno nei file che finirebbero nel repository (tracciati, o nuovi e non esclusi). Si lancia a mano prima di un push:

    python server/strumenti/controlla_dati_personali.py [percorso di personalizzazione.json]

Senza argomento legge OE_PERSONALIZZAZIONE, poi personalizzazione.json nella radice del repository. Senza il file (come
nella Action) non c'è niente da confrontare: esce con 0. Esce con 1 se trova qualcosa.
"""
import json
import os
import subprocess
import sys

RADICE = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))

# Provider generici e voci troppo comuni: non identificano nessuno.
GENERICI = {"gmail.com", "live.it", "live.com", "outlook.it", "outlook.com", "hotmail.it", "hotmail.com",
            "libero.it", "yahoo.it", "yahoo.com", "icloud.com", "eventi personali", "festività", "gmail",
            "libero", "outlook"}
BINARI = (".png", ".ico", ".jpg", ".gif", ".pyc", ".jar", ".asset")
# I file di blocco di npm elencano pacchetti, non dati di nessuno: un nome d'azienda può coincidere con un pacchetto
SALTATI = ("package-lock.json",)


def percorso_personalizzazione(argv):
    if len(argv) > 1:
        return argv[1]
    return os.environ.get("OE_PERSONALIZZAZIONE") or os.path.join(RADICE, "personalizzazione.json")


def valori_da_cercare(p):
    cercati = set()

    def metti(*valori):
        for v in valori:
            s = str(v or "").strip().lower()
            if len(s) >= 5 and s not in GENERICI and not s.startswith(("#", "logos/")):
                cercati.add(s)

    for sezione in ("nomi_per_indirizzo", "nomi_per_dominio", "azienda_per_cartella"):
        for k, v in (p.get(sezione) or {}).items():
            metti(k, v)
    for sezione in ("caselle_di_colleghi", "domini_aziendali", "ordine_calendari", "account_nascosti"):
        metti(*(p.get(sezione) or []))
    for c in p.get("calendari_google") or []:
        metti(c.get("indirizzo"), c.get("nome"))
    metti((p.get("agente_remoto") or {}).get("ssh"))
    for nome, d in (p.get("aziende_inoltrate") or {}).items():
        metti(nome, d.get("via"), d.get("oggetto_posta"))
    ui = p.get("interfaccia") or {}
    metti(*(ui.get("sigle") or {}).keys(), *(ui.get("loghi") or {}).keys(),
          *(ui.get("calendari_secondari") or []), *(ui.get("calendari_senza_teams") or []))
    for s in ui.get("schede_timesheet") or []:
        metti(s.get("nome"), s.get("url"), s.get("sotto"), s.get("azione"))
    return cercati


def file_del_repository():
    """I file che finirebbero nel repository (tracciati o nuovi non esclusi), o None senza git."""
    try:
        out = subprocess.run(["git", "ls-files", "-co", "--exclude-standard"], cwd=RADICE,
                             capture_output=True, text=True, encoding="utf-8", timeout=30)
        if out.returncode == 0:
            return [os.path.join(RADICE, f) for f in out.stdout.splitlines() if f.strip()]
    except (OSError, subprocess.SubprocessError):
        pass
    return None


def main(argv):
    percorso = percorso_personalizzazione(argv)
    try:
        with open(percorso, "r", encoding="utf-8") as f:
            p = json.load(f)
    except FileNotFoundError:
        print("ok   nessuna personalizzazione da confrontare (%s)" % percorso)
        return 0
    except (OSError, ValueError) as e:
        print("KO   personalizzazione illeggibile: %r" % (e,))
        return 1
    cercati = valori_da_cercare(p if isinstance(p, dict) else {})
    file = file_del_repository()
    if file is None:
        print("KO   git non disponibile: controllo non fatto")
        return 1
    trovati = []
    for f in file:
        if not os.path.isfile(f) or f.lower().endswith(BINARI) or os.path.basename(f) in SALTATI:
            continue
        try:
            with open(f, "r", encoding="utf-8") as fh:
                testo = fh.read().lower()
        except (OSError, UnicodeDecodeError):
            continue
        for s in cercati:
            if s in testo:
                trovati.append("%s: %s" % (os.path.relpath(f, RADICE), s))
    if trovati:
        print("KO   dati personali nel codice (%d):" % len(trovati))
        for riga in sorted(trovati):
            print("       " + riga)
        return 1
    print("ok   nessun dato personale nel codice (%d voci cercate in %d file)" % (len(cercati), len(file)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
