"""Avvio dell'app senza finestra di console.

Con pythonw.exe non esiste un terminale su cui stampare: se qualcosa esplode
durante gli import, l'app semplicemente non parte e non si vede nulla. Qui si
cattura tutto e lo si scrive in crash.log, cosi' un avvio fallito lascia sempre
una traccia leggibile.

Le librerie (pywebview, pywin32...) stanno nell'ambiente .venv del progetto, non
nel Python di sistema: aprendo questo file con un doppio clic Windows usa il
Python di sistema e l'app moriva con "No module named 'webview'", lasciando un
processo appeso sulla finestra d'errore. Ora, se ci accorgiamo di non essere
nell'ambiente giusto, ci si riavvia da soli con quello giusto.
"""
import os
import subprocess
import sys
import traceback

CARTELLA = os.path.dirname(os.path.abspath(__file__))
PYTHONW_VENV = os.path.join(CARTELLA, ".venv", "Scripts", "pythonw.exe")


def _rilancia_nel_venv():
    """True se l'avvio e' stato passato all'ambiente del progetto."""
    if os.environ.get("OECOPILOT_RILANCIATO"):
        return False          # gia' rilanciato una volta: non si cicla
    try:
        gia_nel_venv = os.path.normcase(sys.executable).startswith(
            os.path.normcase(os.path.join(CARTELLA, ".venv")))
        if gia_nel_venv or not os.path.exists(PYTHONW_VENV):
            return False
        ambiente = dict(os.environ, OECOPILOT_RILANCIATO="1")
        subprocess.Popen([PYTHONW_VENV, os.path.abspath(__file__)],
                         cwd=CARTELLA, env=ambiente, close_fds=True)
        return True
    except Exception:
        return False


if _rilancia_nel_venv():
    sys.exit(0)

try:
    import app
    app.main()
except Exception:
    percorso = os.path.join(CARTELLA, "crash.log")
    with open(percorso, "w", encoding="utf-8") as f:
        f.write(traceback.format_exc())
    try:
        import ctypes
        ctypes.windll.user32.MessageBoxW(
            0,
            "OE Copilot non è riuscito ad avviarsi.\n\n"
            f"Il dettaglio dell'errore è in:\n{percorso}",
            "OE Copilot", 0x10)
    except Exception:
        pass
