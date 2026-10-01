"""Installa in pywebview 6.2.1 le due modifiche usate da OE Copilot.

pywebview, sotto Windows, ha due limiti che l'app aggira modificando un suo file
(webview/platforms/winforms.py):

  1. l'icona dell'app va impostata prima che la finestra crei il suo handle,
     altrimenti la barra delle applicazioni mostra l'icona di Python;
  2. una finestra senza cornice non si ridimensiona dai bordi e, ingrandita,
     lascia un margine vuoto attorno al contenuto.

Le modifiche sono gia' dentro pywebview-6.2.1-winforms.py (cerca "PATCH"):
questo script controlla la versione installata, conserva l'originale come
winforms.py.originale e mette al suo posto la versione modificata.
pywebview e' distribuito con licenza BSD (vedi LICENSE-pywebview.txt).

Uso, dalla cartella dell'app, dopo pip install -r requirements.txt:

    .venv\\Scripts\\python.exe strumenti\\applica_patch_pywebview.py
"""
import os
import shutil
import sys
from importlib.metadata import PackageNotFoundError, version

QUI = os.path.dirname(os.path.abspath(__file__))
SORGENTE = os.path.join(QUI, "pywebview-6.2.1-winforms.py")
VERSIONE = "6.2.1"


def main():
    try:
        installata = version("pywebview")
    except PackageNotFoundError:
        print("pywebview non e' installato: prima pip install -r requirements.txt")
        return 1
    if installata != VERSIONE:
        print("pywebview %s installato, la modifica e' per la %s:\n"
              "  pip install pywebview==%s" % (installata, VERSIONE, VERSIONE))
        return 1
    # Il percorso si ricava senza importare winforms (che caricherebbe .NET).
    import webview
    destinazione = os.path.join(os.path.dirname(webview.__file__), "platforms", "winforms.py")
    with open(SORGENTE, "rb") as f:
        nuovo = f.read()
    with open(destinazione, "rb") as f:
        attuale = f.read()
    if attuale == nuovo:
        print("Modifica gia' applicata:", destinazione)
        return 0
    copia = destinazione + ".originale"
    if not os.path.exists(copia):
        shutil.copy2(destinazione, copia)
    shutil.copy2(SORGENTE, destinazione)
    print("Modifica applicata a", destinazione)
    print("Originale conservato in", copia)
    return 0


if __name__ == "__main__":
    sys.exit(main())
