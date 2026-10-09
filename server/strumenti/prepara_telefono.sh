#!/bin/bash
# Mette il telefono virtuale nello stato che serve, ogni volta allo stesso modo:
# fuso orario e lingua, schermo sempre acceso, app Outlook installata e niente
# risparmio batteria per le app che sincronizzano.
#
# Si può rilanciare quanto si vuole: fa solo quello che manca. Lo lancia
# aggiorna_server.sh a ogni aggiornamento, quindi un telefono ricreato da zero
# torna a posto da solo (restano da rifare solo i login).
#
#   FUSO=Europe/Rome LINGUA=it-IT prepara_telefono.sh
set -euo pipefail
DIR="${OE_DIR:-/opt/oe-copilot}"
T=oe-copilot-telefono
FUSO=${FUSO:-Europe/Rome}
LINGUA=${LINGUA:-it-IT}
OUTLOOK=com.microsoft.office.outlook
# App che non devono finire in risparmio batteria (se sono installate).
SVEGLIE="$OUTLOOK at.bitfire.davdroid"

e() { docker exec "$T" "$@"; }

[ "$(e getprop sys.boot_completed 2>/dev/null)" = 1 ] \
    || { echo "il telefono non è avviato" >&2; exit 1; }

e setprop persist.sys.timezone "$FUSO"
e settings put global auto_time_zone 0
e setprop persist.sys.locale "$LINGUA"
e settings put system system_locales "$LINGUA"
# "svc power stayon true" qui non basta: redroid non ha una batteria in carica
# e l'impostazione resta a 0. Scritta direttamente tiene.
e settings put global stay_on_while_plugged_in 7

# Outlook esiste solo sul Play Store e il telefono non ha i servizi Google:
# l'APK sta fra i file della release "apk" del repository e arriva in apk/ con
# l'aggiornamento. Si installa solo quello elencato in apk/SHA256SUMS, e solo se
# l'impronta coincide; che sia davvero firmato da Microsoft lo controlla Android.
if ! e pm list packages | grep -qx "package:$OUTLOOK"; then
    apk=$(awk '$2 ~ /^\*?Outlook-.*\.apk$/ {sub(/^\*/, "", $2); print $2}' "$DIR/apk/SHA256SUMS" | head -1)
    if [ -n "$apk" ] && [ -f "$DIR/apk/$apk" ]; then
        (cd "$DIR/apk" && grep -F "$apk" SHA256SUMS | sha256sum -c --quiet -)
        docker cp "$DIR/apk/$apk" "$T:/data/local/tmp/outlook.apk" >/dev/null
        e pm install -r /data/local/tmp/outlook.apk >/dev/null
        e rm -f /data/local/tmp/outlook.apk
        echo "installato Outlook da $apk"
    else
        echo "Outlook non installato e APK assente in $DIR/apk (vedi GUIDA.md, 4.4)" >&2
    fi
fi

installate=$(e pm list packages)
for app in $SVEGLIE; do
    if grep -qx "package:$app" <<<"$installate"; then
        e dumpsys deviceidle whitelist "+$app" >/dev/null
    fi
done

versione=$(e dumpsys package "$OUTLOOK" | sed -n 's/^ *versionName=//p' | head -1 || true)
echo "telefono pronto: fuso $(e getprop persist.sys.timezone), lingua $(e getprop persist.sys.locale)," \
     "schermo acceso=$(e settings get global stay_on_while_plugged_in), Outlook ${versione:-assente}"
