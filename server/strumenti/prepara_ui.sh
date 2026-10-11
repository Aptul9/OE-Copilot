#!/bin/bash
# Oe Copilot - prepara la cartella da cui si costruisce l'immagine ui: i moduli dell'app (radice del
# repository) che il servizio ui usa così come sono, e i loghi (web/logos). Sono copie e non
# stanno nel repository (server/ui/app/ è in .gitignore): la sorgente resta una sola.
# Lo lanciano la Action (prima del controllo e prima di caricare il codice) e chi vuole
# provare l'immagine in locale.
set -euo pipefail
cd "$(dirname "$0")/../.."
rm -rf server/ui/app
mkdir -p server/ui/app
cp app.py config.py caselle.py eventi.py disponibilita.py agente_remoto.py ics_feed.py \
   local_events.py server/ui/app/
mkdir -p server/ui/app/web
cp -r web/logos server/ui/app/web/logos
