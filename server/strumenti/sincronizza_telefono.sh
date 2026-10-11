#!/bin/bash
# Ogni 30 secondi tiene aggiornati i calendari del telefono virtuale. Da solo
# Android sincronizzerebbe al massimo ogni 15 minuti.
#
# - Outlook senza i servizi Google scaricava dal server SOLO con la schermata
#   principale davanti (prove del 05/10/2026: fermo nelle impostazioni nessuna
#   riunione nuova in 6 minuti, in primo piano ~20 s). Con i servizi Google riceve
#   le notifiche push (FCM), ma sono solo quelle della posta: a telefono bloccato
#   una cancellazione o una modifica di un appuntamento non ha push e il telefono
#   la applica solo quando Outlook è davvero davanti, cioè con il telefono sbloccato
#   (11/10/2026: 7 inviti cancellati alle 09:45 UTC arrivati sul telefono alle 11:42:29,
#   appena sbloccato; a telefono bloccato ogni giro porta Outlook davanti per 15 ms
#   e Android lo rimette in pausa, senza scaricare niente). A ogni giro lo si rimette
#   davanti quando lo schermo lo permette. Si forzano anche i lavori
#   di sincronizzazione di Android per ogni casella (cmd jobscheduler run -f), che
#   copiano subito nel calendario del telefono; gli ID si rileggono a ogni giro
#   perché cambiano quando si aggiunge una casella o si riavvia il telefono.
# - Telefono sbloccato: il criterio del dispositivo di Outlook (Serco) limita il tempo
#   di schermo acceso a 15 minuti senza input (mMaximumScreenOffTimeoutFromDeviceAdmin,
#   forzato: anche lo "schermo sempre acceso" di Android viene ignorato) e poi il
#   telefono si blocca di nuovo. Ogni TOCCO_OGNI giri (10 = 5 minuti, 0 spegne), se
#   lo schermo è acceso, un tocco sulla barra di stato (nessuna app lo riceve) conta
#   come attività dell'utente e riparte il conteggio: dopo il PIN il telefono resta
#   sbloccato e Outlook resta davanti. Un tasto da shell non basta (provato: il
#   tocco aggiorna mLastUserActivityTime, il tasto no). A schermo spento il tocco non
#   si manda.
# - Senza blocco dello schermo (credenziale tolta l'11/10/2026 con "cmd lock_settings
#   clear": nessuna schermata di blocco, nemmeno dopo aver spento lo schermo): se lo
#   schermo è spento (tasto di accensione della pagina, o il tocco mancato) e nessuna
#   pagina è aperta, lo si riaccende prima di portare Outlook davanti: a schermo spento
#   Outlook resta in pausa e non scarica niente. Con un blocco (PIN) il tasto
#   accenderebbe solo la schermata di blocco: se si rimette un PIN, lo digita sempre l'utente.
# - Calendari Google (DAVx5): DAVx5 usa WorkManager, che scarta le esecuzioni
#   anticipate dei lavori periodici; si usa la sua scorciatoia ufficiale
#   "Sincronizzazione di tutti gli account" (azione android.intent.action.SYNC),
#   che porta DAVx5 in primo piano per un attimo, poi si rimette Outlook davanti.
#
# Se esiste il file PAUSA (es. durante un login dalla finestra scrcpy) non si
# tocca lo schermo: si forzano solo le sincronizzazioni di Android. Vale anche
# stato/schermo/PAUSA, che il servizio dello schermo nel browser (schermo-web) riscrive ogni 10
# secondi finché c'è una pagina aperta: se è più vecchio di 60 secondi si ignora, così un
# servizio morto non ferma la sincronizzazione.
#
# A ogni giro lascia un segno di vita in stato/battito.json, che l'agente passa
# all'app: se il servizio si ferma o il telefono è spento, l'app lo dice invece
# di mostrare per buoni dati fermi. Che Outlook risponda casella per casella e
# che le sue push funzionino lo controlla rileva_accessi.py (stato/accessi.json).
T=oe-copilot-telefono
OGNI=${OGNI:-30}
PAUSA=/opt/oe-copilot/telefono/PAUSA
STATO=/opt/oe-copilot/stato
PAUSA_SCHERMO=$STATO/schermo/PAUSA
PAUSA_SCHERMO_SCADE=${PAUSA_SCHERMO_SCADE:-60}
# Ogni quanti giri rilevare accessi scaduti e freschezza (un minuto): il controllo
# copia due piccoli database e legge la coda di due log, costa poco.
ACCESSI_OGNI=${ACCESSI_OGNI:-2}
# Ogni quanti giri forzare il sincronizzatore dei calendari Google: si sveglia da
# solo con le notifiche push, forzarlo copre il caso che una notifica non arrivi.
GOOGLE_OGNI=${GOOGLE_OGNI:-4}
# Ogni quanti giri un tocco sulla barra di stato tiene sveglio un telefono sbloccato (0 = mai).
TOCCO_OGNI=${TOCCO_OGNI:-10}
mkdir -p "$STATO"
# La cartella del file PAUSA del servizio schermo (utente 10001): la crea questo servizio, che gira da root. L'utente dell'aggiornamento
# automatico non ha sudo, e se la cartella non c'è Docker la crea di root e il servizio schermo non ci scrive.
install -d -o 10001 -g 10001 -m 755 "$STATO/schermo"

conta=0
while true; do
    telefono=false; davanti=false; pausa=false
    [ -e "$PAUSA" ] && pausa=true
    if [ -f "$PAUSA_SCHERMO" ] && [ $(( $(date +%s) - $(stat -c %Y "$PAUSA_SCHERMO" 2>/dev/null || echo 0) )) -lt "$PAUSA_SCHERMO_SCADE" ]; then
        pausa=true
    fi
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
            docker exec "$T" dumpsys power 2>/dev/null | grep -q 'mWakefulness=Awake' \
                || { docker exec "$T" input keyevent 224 >/dev/null 2>&1; sleep 1; }
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
        if [ "$TOCCO_OGNI" -gt 0 ] && [ $((conta % TOCCO_OGNI)) -eq 0 ] \
           && docker exec "$T" dumpsys power 2>/dev/null | grep -q 'mWakefulness=Awake'; then
            docker exec "$T" input tap 360 5 >/dev/null 2>&1
        fi
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
