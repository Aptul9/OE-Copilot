#!/bin/bash
# Oe Copilot - prepara la cartella da cui si costruisce l'immagine ui: i moduli e la
# pagina dell'app del PC, che l'interfaccia web riusa così come sono. Sono copie e non
# stanno nel repository (server/ui/app/ è in .gitignore): la sorgente resta una sola.
# Lo lanciano la Action (prima del controllo e prima di caricare il codice) e chi vuole
# provare l'immagine in locale.
set -euo pipefail
cd "$(dirname "$0")/../.."
rm -rf server/ui/app
mkdir -p server/ui/app
cp app.py config.py caselle.py eventi.py disponibilita.py agente_remoto.py ics_feed.py \
   local_events.py outlook_com.py server/ui/app/
cp -r web server/ui/app/web
