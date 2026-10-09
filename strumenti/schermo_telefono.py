"""Apre sul PC lo schermo del telefono virtuale che sta sul server.

Serve per installare le app e fare i login (password e MFA li scrive chi usa
l'app, nella finestra). Fa da solo tutto il contorno:

  1. crea il file PAUSA sul server, così il servizio di sincronizzazione non
     porta Outlook in primo piano mentre si sta scrivendo;
  2. apre un tunnel SSH verso la porta adb del telefono (che sul server ascolta
     solo su localhost) e ci collega adb;
  3. lancia scrcpy; alla chiusura della finestra toglie PAUSA e chiude il tunnel.

Uso:
  .venv\\Scripts\\python strumenti\\schermo_telefono.py [utente@server]

Senza indirizzo usa quello di "agente_remoto" in personalizzazione.json.

Servono scrcpy e adb sul PC (scoop install scrcpy, oppure lo zip di scrcpy,
che contiene anche adb).
"""
import argparse
import os
import shutil
import socket
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

PORTA_LOCALE = 15555
PORTA_ADB = 5555
PAUSA = "/opt/oe-copilot/telefono/PAUSA"
DISPOSITIVO = "127.0.0.1:%d" % PORTA_LOCALE
SENZA_FINESTRA = getattr(subprocess, "CREATE_NO_WINDOW", 0)


def _trova(nome):
    """scrcpy e adb: nel PATH, oppure accanto a scrcpy installato con scoop
    (che mette nel PATH solo scrcpy)."""
    trovato = shutil.which(nome)
    if trovato:
        return trovato
    scoop = os.path.join(os.path.expanduser("~"), "scoop", "apps", "scrcpy",
                         "current", nome + ".exe")
    return scoop if os.path.exists(scoop) else None


def _porta_aperta():
    with socket.socket() as s:
        s.settimeout(0.5)
        return s.connect_ex(("127.0.0.1", PORTA_LOCALE)) == 0


def _ssh(server, comando):
    return subprocess.run(["ssh", "-o", "BatchMode=yes", server, comando],
                          capture_output=True, text=True,
                          creationflags=SENZA_FINESTRA)


def main():
    ap = argparse.ArgumentParser(description="Schermo del telefono virtuale del server.")
    ap.add_argument("server", nargs="?", help="utente@server o alias di ~/.ssh/config")
    args = ap.parse_args()

    server = args.server
    if not server:
        import config
        server = str(config.AGENTE_REMOTO.get("ssh") or "").strip()
    if not server:
        sys.exit("Indirizzo del server mancante: passalo come argomento "
                 "o mettilo in agente_remoto.ssh della personalizzazione.")

    scrcpy, adb = _trova("scrcpy"), _trova("adb")
    if not scrcpy or not adb:
        sys.exit("scrcpy e adb non trovati: installa scrcpy (scoop install scrcpy).")
    if _porta_aperta():
        sys.exit("La porta %d è già in uso: c'è un altro tunnel o un'altra finestra aperta."
                 % PORTA_LOCALE)

    r = _ssh(server, "sudo -n touch " + PAUSA)
    if r.returncode != 0:
        sys.exit("Il server non risponde o PAUSA non si può creare: " + r.stderr.strip())
    print("Sincronizzazione dello schermo in pausa sul server.")

    tunnel = subprocess.Popen(
        ["ssh", "-N", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes",
         "-L", "%d:127.0.0.1:%d" % (PORTA_LOCALE, PORTA_ADB), server],
        creationflags=SENZA_FINESTRA)
    try:
        for _ in range(40):
            if _porta_aperta():
                break
            if tunnel.poll() is not None:
                sys.exit("Il tunnel SSH non si è aperto.")
            time.sleep(0.25)
        subprocess.run([adb, "connect", DISPOSITIVO], capture_output=True)
        subprocess.run([adb, "-s", DISPOSITIVO, "wait-for-device"], timeout=30)
        print("Finestra del telefono aperta: chiudila quando hai finito.")
        subprocess.run([scrcpy, "-s", DISPOSITIVO, "--window-title", "Telefono OE Copilot"])
    finally:
        subprocess.run([adb, "disconnect", DISPOSITIVO], capture_output=True)
        tunnel.terminate()
        r = _ssh(server, "sudo -n rm -f " + PAUSA)
        print("Sincronizzazione ripresa." if r.returncode == 0
              else "ATTENZIONE: PAUSA non rimosso sul server (%s): toglilo a mano." % r.stderr.strip())


if __name__ == "__main__":
    main()
