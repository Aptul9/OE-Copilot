#!/usr/bin/env python3
"""Prova dei loghi delle aziende: un server vero su una porta libera, una cartella dati finta. La lancia la Action.

Si esegue nell'immagine ui (ha le librerie e i moduli in /app, server.py è in /app/server.py):
    docker run --rm -v "$PWD/server/strumenti/prova_loghi.py:/prova.py:ro" oe-copilot-ui python /prova.py
"""
import os
import sys
import tempfile
import threading
import urllib.error
import urllib.request

dati = tempfile.mkdtemp()
os.makedirs(os.path.join(dati, "logos"))
PNG = bytes.fromhex("89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000001e221bc330000000049454e44ae426082")
with open(os.path.join(dati, "logos", "acme.png"), "wb") as f:
    f.write(PNG)
with open(os.path.join(dati, "segreto.txt"), "w") as f:
    f.write("fuori dalla cartella dei loghi")
os.environ["OE_DATI"] = dati

qui = os.path.dirname(os.path.abspath(__file__))
# nell'immagine app.py e server.py stanno insieme in /app; nel repository server.py è in server/ui e i moduli copiati
# da prepara_ui.sh in server/ui/app
if os.path.exists("/app/server.py"):
    sys.path.insert(0, "/app")
else:
    sys.path[0:0] = [os.path.abspath(os.path.join(qui, "..", "ui")), os.path.abspath(os.path.join(qui, "..", "ui", "app"))]
import server  # noqa: E402

httpd = server.ThreadingHTTPServer(("127.0.0.1", 0), server.Gestore)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
base = "http://127.0.0.1:%d" % httpd.server_address[1]
esiti = 0


def prendi(percorso):
    try:
        r = urllib.request.urlopen(base + percorso, timeout=5)
        return r.status, r.headers.get("Content-Type"), r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers.get("Content-Type"), b""


def verifica(nome, ok, dettaglio=""):
    global esiti
    print(("ok   " if ok else "KO   ") + nome + ("" if ok else "  -> " + str(dettaglio)))
    esiti += 0 if ok else 1


s, t, c = prendi("/logos/acme.png")
verifica("un logo della cartella dati si serve", s == 200 and t == "image/png" and c == PNG, (s, t, len(c)))
s, t, c = prendi("/logos/personale.png")
verifica("il logo che sta già nella pagina si serve ancora", s == 200 and t == "image/png", (s, t))
verifica("un logo che non c'è dà 404", prendi("/logos/nessuno.png")[0] == 404)
for p in ("/logos/../segreto.txt", "/logos/%2e%2e/segreto.txt", "/logos/..%2fsegreto.txt", "/logos//etc/passwd", "/logos/%2e%2e/%2e%2e/etc/passwd"):
    s, _, c = prendi(p)
    verifica("%s non esce dalla cartella" % p, s == 404 and b"fuori" not in c, s)
httpd.shutdown()
print("\nESITO: %s" % ("tutto ok" if not esiti else "%d KO" % esiti))
sys.exit(1 if esiti else 0)
