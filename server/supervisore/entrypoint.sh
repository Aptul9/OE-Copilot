#!/bin/sh
# Avvia il server adb (il supervisore lo raggiunge su 127.0.0.1:5037 e collega da sé il telefono di TELEFONO, anche dopo un riavvio),
# poi il supervisore.
adb start-server >/dev/null 2>&1
exec node dist/index.js
