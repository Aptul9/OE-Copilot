#!/bin/bash
# Ogni 30 secondi tiene aggiornati i calendari del telefono virtuale. Da solo
# Android sincronizzerebbe al massimo ogni 15 minuti.
#
# - Outlook senza i servizi Google scaricava dal server SOLO con la schermata
#   principale davanti (prove del 05/10/2026: fermo nelle impostazioni nessuna
#   riunione nuova in 6 minuti, in primo piano ~20 s). Con i servizi Google riceve
#   le notifiche push (FCM) e scarica anche a telefono bloccato (verificato
#   l'11/10/2026: push ricevuti a schermo bloccato, appuntamenti riscritti nel
#   calendario subito dopo). A ogni giro lo si rimette comunque davanti quando lo
#   schermo lo permette: copre il caso di una push persa. Si forzano anche i lavori
#   di sincronizzazione di Android per ogni casella (cmd jobscheduler run -f), che
#   copiano subito nel calendario del telefono; gli ID si rileggono a ogni giro
#   perché cambiano quando si aggiunge una casella o si riavvia il telefono.
# - Calendari Google (DAVx5): DAVx5 usa WorkManager, che scarta le esecuzioni
#   anticipate dei lavori periodici; si usa la sua scorciatoia ufficiale
#   "Sincronizzazione di tutti gli account" (azione android.intent.action.SYNC),
#   che porta DAVx5 in primo piano per un attimo, poi si rimette Outlook davanti.
#
# Se esiste il file PAUSA (es. durante un login dalla finestra scrcpy) non si
# tocca lo schermo: si forzano solo le sincronizzazioni di Android.
#
# A ogni giro lascia un segno di vita in stato/battito.json, che l'agente passa
# all'app: se il servizio si ferma o il telefono è spento, l'app lo dice invece
# di mostrare per buoni dati fermi. Che Outlook risponda casella per casella e
# che le sue push funzionino lo controlla rileva_accessi.py (stato/accessi.json).
T=oe-copilot-telefono
OGNI=${OGNI:-30}
PAUSA=/opt/oe-copilot/telefono/PAUSA
STATO=/opt/oe-copilot/stato
# Ogni quanti giri rilevare accessi scaduti e freschezza (un minuto): il controllo
# copia due piccoli database e legge la coda di due log, costa poco.
ACCESSI_OGNI=${ACCESSI_OGNI:-2}
# Ogni quanti giri forzare il sincronizzatore dei calendari Google: si sveglia da
# solo con le notifiche push, forzarlo copre il caso che una notifica non arrivi.
GOOGLE_OGNI=${GOOGLE_OGNI:-4}
mkdir -p "$STATO"

conta=0
while true; do
    telefono=false; davanti=false; pausa=false
    [ -e "$PAUSA" ] && pausa=true
    if docker exec "$T" true 2>/dev/null; then
        telefono=true
        lavori=$(docker exec "$T" dumpsys jobscheduler 2>/dev/null)
        # Outlook e Google si sincronizzano con gli stessi lavori di Android: si
        # forzano sempre quelli di Outlook e, ogni GOOGLE_OGNI giri, quelli di Google.
        modelli='com\.microsoft\.office\.outlook'
        [ $((conta % GOOGLE_OGNI)) -eq 0 ] && modelli="$modelli|com\.google"
        for id in $(grep -oE "JOB #1000/[0-9]+: [0-9a-f]+ com\.android\.calendar/($modelli)" <<<"$lavori" \
                    | sed -E 's|JOB #1000/([0-9]+).*|\1|' | sort -u); do
            docker exec "$T" cmd jobscheduler run -f android "$id" >/dev/null 2>&1
        done
        if [ "$pausa" = false ]; then
            docker exec "$T" am start -a android.intent.action.SYNC \
                -n at.bitfire.davdroid/.ui.AccountsActivity >/dev/null 2>&1
            sleep 2
            docker exec "$T" am start -a android.intent.action.MAIN \
                -c android.intent.category.LAUNCHER \
                -n com.microsoft.office.outlook/.MainActivity >/dev/null 2>&1
            sleep 2
            # Outlook torna sull'ultima schermata aperta: se e' rimasto nelle
            # impostazioni (o altrove) non scarica niente. In quel caso lo si
            # riapre da capo sulla vista principale (NEW_TASK|CLEAR_TASK).
            fuoco=$(docker exec "$T" dumpsys window 2>/dev/null | grep -m1 mCurrentFocus)
            case "$fuoco" in
                *com.acompli.acompli.CentralActivity*) ;;
                *) docker exec "$T" am start -f 0x10008000 \
                       -n com.microsoft.office.outlook/.MainActivity >/dev/null 2>&1
                   sleep 3 ;;
            esac
        fi
        fuoco=$(docker exec "$T" dumpsys window 2>/dev/null | grep -m1 mCurrentFocus)
        case "$fuoco" in *com.acompli.acompli.CentralActivity*) davanti=true ;; esac
    fi
    # Scrittura atomica: l'agente non deve mai leggere un file a meta'.
    printf '{"quando": %s, "pausa": %s, "telefono": %s, "outlook_davanti": %s}\n' \
        "$(date +%s)" "$pausa" "$telefono" "$davanti" > "$STATO/battito.json.tmp" \
        && mv -f "$STATO/battito.json.tmp" "$STATO/battito.json"
    # Ogni ACCESSI_OGNI giri: controlla se qualche accesso e' scaduto sul
    # telefono (altrimenti il pallino resterebbe verde su dati fermi).
    if [ "$telefono" = true ] && [ $((conta % ACCESSI_OGNI)) -eq 0 ]; then
        python3 /opt/oe-copilot/strumenti/rileva_accessi.py >/dev/null 2>&1 || true
    fi
    conta=$((conta + 1))
    sleep "$OGNI"
done
