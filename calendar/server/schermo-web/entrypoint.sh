#!/bin/sh
# Collega adb al telefono sulla rete di compose e riprova ogni 15 secondi (dopo un riavvio del telefono la
# connessione cade), poi avvia il servizio. Un solo telefono: quello di TELEFONO.
adb start-server >/dev/null 2>&1
(
    while true; do
        adb connect "$TELEFONO" >/dev/null 2>&1
        sleep 15
    done
) &
exec node index.js
