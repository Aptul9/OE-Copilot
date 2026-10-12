#!/bin/bash
# Oe Copilot - aggiornamento del server, lanciato dalla GitHub Action via SSH.
#   aggiorna_server.sh salva        mette da parte il codice attuale (per tornare indietro)
#   aggiorna_server.sh avvia <sha>  ricostruisce e riavvia; se un passo non riesce o l'agente non risponde torna alla versione salvata
# Il codice arriva con rsync dalla Action; .env, agent/fonti.json, telefono/, stato/ e
# ui-dati/ restano sul server e non vengono toccati.
set -euo pipefail
DIR="${OE_DIR:-/opt/oe-copilot}"
COPIA=/var/tmp/oe-copilot-prima.tgz
SUPERVISORE=oe-copilot-supervisore
T=oe-copilot-telefono
cd "$DIR"

# Gira dentro `if avvia && sano`: lì errexit è spento, quindi ogni passo che può fallire ha il suo `|| return 1` e
# l'ultimo comando (il supervisore) dà lo stato di ritorno della funzione.
avvia() {
    lsmod | grep -q '^binder_linux' \
        || { echo "modulo binder_linux non caricato (vedi GUIDA.md, 4.1)" >&2; return 1; }
    # Il telefono si ricrea solo se la sua configurazione è cambiata: un riavvio
    # inutile fermerebbe le sincronizzazioni per un paio di minuti.
    docker compose up -d --build telefono \
        || { echo "docker compose non ha avviato il telefono" >&2; return 1; }
    # L'agente monta la cartella del calendario del telefono: deve esistere prima,
    # altrimenti Docker la crea vuota e di root al posto di Android.
    for _ in $(seq 1 60); do
        [ "$(docker exec "$T" getprop sys.boot_completed 2>/dev/null)" = 1 ] && break
        sleep 5
    done
    [ "$(docker exec "$T" getprop sys.boot_completed 2>/dev/null)" = 1 ] \
        || { echo "il telefono non ha finito di avviarsi" >&2; return 1; }
    # impostazioni del telefono e negozio delle app: stanno nello script, non a mano
    "$DIR/strumenti/prepara_telefono.sh" \
        || { echo "prepara_telefono.sh non è riuscito" >&2; return 1; }
    # Il codice dell'agente è nell'immagine, niente file montati: compose lo ricrea da sé quando l'immagine o la configurazione cambiano.
    docker compose up -d --build agent \
        || { echo "docker compose non ha avviato l'agente" >&2; return 1; }
    # Il giro di sincronizzazione, il battito e il rilevatore degli accessi li fa il supervisore (supervisore/, GUIDA.md 4.5), che si
    # ricrea a ogni aggiornamento senza toccare il telefono (--no-deps).
    riavvio=$(date +%s)
    docker compose up -d --build --force-recreate --no-deps supervisore
}

# Email e password del proprietario, chiave dell'API e segreto dei cookie possono arrivare dalla Action sullo standard
# input (righe NOME=valore), invece che da .env: compose legge prima l'ambiente della shell, quindi bastano esportate.
# Un valore vuoto non sovrascrive niente.
credenziali_da_stdin() {
    [ -t 0 ] && return 0
    local riga nome valore
    while IFS= read -r riga || [ -n "$riga" ]; do
        nome=${riga%%=*}; valore=${riga#*=}
        case "$nome" in ADMIN_EMAIL|ADMIN_PASSWORD|API_KEY|BETTER_AUTH_SECRET) [ -z "$valore" ] || export "$nome=$valore" ;; esac
    done
}

# Interfaccia web (facoltativa, GUIDA.md 4.7): il gateway, che chiede l'accesso e serve la
# pagina, l'API e i loghi, e il Caddy che le mette HTTPS davanti. Parte solo se in .env c'è UI_HOST.
# Gira dopo che agente e battito sono a posto e non fa tornare indietro niente: se la web
# non parte, la Action diventa rossa ma l'agente e il telefono restano come sono. Il Caddy
# nuovo si ricrea solo se il gateway è sano (depends_on): se non lo è resta quello di prima.
avvia_ui() {
    grep -q '^UI_HOST=.' .env || { echo "interfaccia web non configurata (UI_HOST in .env): salto"; return 0; }
    [ -d ui-dati ] || { echo "manca $DIR/ui-dati (vedi GUIDA.md, 4.7)" >&2; return 1; }
    for v in ADMIN_EMAIL ADMIN_PASSWORD BETTER_AUTH_SECRET; do
        [ -n "${!v:-}" ] || grep -q "^$v=." .env \
            || { echo "manca $v (né nei secret della Action né in $DIR/.env, vedi GUIDA.md, 4.7): l'interfaccia web resta com'è" >&2; return 1; }
    done
    docker compose up -d --build gateway caddy-ui \
        || { echo "docker compose non ha avviato gateway e caddy-ui" >&2; docker logs --tail 40 oe-copilot-gateway >&2 || true; return 1; }
    # Il servizio ui non c'è più. Il suo container e la sua immagine, rimasti dalle versioni di prima, si tolgono solo
    # adesso: il gateway vecchio gli chiedeva i loghi fino a questo punto, quello nuovo (sano) fa da sé.
    if docker container inspect oe-copilot-ui >/dev/null 2>&1; then
        docker rm -f oe-copilot-ui >/dev/null && echo "tolto il container oe-copilot-ui: il servizio ui non c'è più"
    fi
    docker image rm oe-copilot-ui >/dev/null 2>&1 || true
    stato=""
    for _ in $(seq 1 30); do
        stato="$(docker inspect -f '{{.State.Health.Status}}' oe-copilot-gateway 2>/dev/null)"
        [ "$stato" = healthy ] && break
        sleep 2
    done
    [ "$stato" = healthy ] || { echo "il servizio oe-copilot-gateway non risponde" >&2; docker logs --tail 40 oe-copilot-gateway >&2 || true; return 1; }
    # Il servizio agente-ts non c'è più: l'agente in TypeScript è `agent`. Il container e l'immagine oe-copilot-agente-ts di prima si
    # tolgono solo se il gateway sano non li usa (AGENTE_URL vuoto o quello di partenza, http://agent:8180); con un AGENTE_URL che
    # punta altrove restano, e il motivo finisce nel log.
    if docker container inspect oe-copilot-agente-ts >/dev/null 2>&1; then
        if url="$(docker exec oe-copilot-gateway printenv AGENTE_URL 2>/dev/null)"; then
            case "$url" in
                ""|http://agent:8180)
                    docker rm -f oe-copilot-agente-ts >/dev/null && echo "tolto il container oe-copilot-agente-ts: il servizio agente-ts non c'è più, l'agente è agent"
                    docker image rm oe-copilot-agente-ts >/dev/null 2>&1 || true ;;
                *) echo "container oe-copilot-agente-ts lasciato: il gateway usa AGENTE_URL=$url, non http://agent:8180" ;;
            esac
        else
            echo "container oe-copilot-agente-ts lasciato: AGENTE_URL non si legge dal gateway" >&2
        fi
    fi
    # il cancello deve chiudere davvero: senza sessione la pagina rimanda a /login e l'API risponde 401
    [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 http://127.0.0.1:3190/logos/personale.png)" = 302 ] \
        && [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 -X POST -H 'Content-Type: application/json' -d '[]' http://127.0.0.1:3190/api/get_config)" = 401 ] \
        || { echo "il gateway non chiude l'accesso come dovrebbe" >&2; return 1; }
    # Il Caddy: `compose up` dà 0 anche se il suo container si riavvia in continuazione, e con un gateway sano il sito resterebbe spento dietro sslh
    # con la Action verde. Una richiesta HTTPS alla sua porta pubblicata (4445) con il nome del sito (-k: il certificato può essere non fidato; se
    # manca del tutto, come alla prima installazione prima che Caddy lo ottenga, la risposta non arriva e il controllo fallisce: si rilancia dopo
    # qualche minuto) deve dare 200, e il container non deve essersi riavviato.
    ui_host="${UI_HOST:-$(grep '^UI_HOST=' .env | tail -n 1 | cut -d= -f2- | tr -d "\r\"'")}"
    codice=""
    for _ in $(seq 1 15); do
        codice="$(curl -ksS --max-time 5 --resolve "$ui_host:4445:127.0.0.1" -o /dev/null -w '%{http_code}' "https://$ui_host:4445/login" 2>/dev/null)" || true
        [ "$codice" = 200 ] && break
        sleep 2
    done
    riavvii="$(docker inspect -f '{{.RestartCount}}' oe-copilot-caddy 2>/dev/null)" || riavvii="?"
    if [ "$codice" != 200 ] || [ "$riavvii" != 0 ]; then
        echo "il servizio oe-copilot-caddy non serve il sito (https://$ui_host:4445/login: ${codice:-nessuna risposta}, riavvii del container: $riavvii)" >&2
        docker logs --tail 40 oe-copilot-caddy >&2 || true
        return 1
    fi
}

# Schermo del telefono nel browser (schermo-web, GUIDA.md 4.8). Come la web: se non parte la Action diventa rossa,
# agente e telefono restano come sono. Il telefono non si tocca: si costruisce e si avvia solo questo servizio.
avvia_schermo() {
    docker compose up -d --build schermo
    stato=""
    for _ in $(seq 1 30); do
        stato="$(docker inspect -f '{{.State.Health.Status}}' oe-copilot-schermo 2>/dev/null)"
        [ "$stato" = healthy ] && break
        sleep 2
    done
    [ "$stato" = healthy ] || { echo "il servizio oe-copilot-schermo non risponde" >&2; docker logs --tail 40 oe-copilot-schermo >&2 || true; return 1; }
    # la cartella del file PAUSA (stato/schermo, utente 10001, si crea una volta a mano: GUIDA.md 4.5) deve essere scrivibile dal container: senza,
    # la sincronizzazione continuerebbe a portare Outlook in primo piano mentre si usa il telefono nel browser
    docker exec oe-copilot-schermo sh -c 'touch /pausa/.prova && rm /pausa/.prova' \
        || { echo "stato/schermo non è scrivibile dal servizio schermo (sudo install -d -o 10001 -g 10001 -m 755 /opt/oe-copilot/stato/schermo)" >&2; return 1; }
}

sano() {
    chiave="${API_KEY:-$(grep '^API_KEY=' .env | cut -d= -f2)}"
    for _ in $(seq 1 20); do
        if curl -fsS --max-time 5 -H "Authorization: Bearer $chiave" \
                http://127.0.0.1:3180/api/salute >/dev/null 2>&1; then
            # il servizio scrive il battito a ogni giro: ne serve uno scritto dopo il
            # riavvio, quello di prima non dice niente
            for _ in $(seq 1 30); do
                [ "$(stat -c %Y stato/battito.json 2>/dev/null || echo 0)" -ge "$riavvio" ] \
                    && [ "$(docker inspect -f '{{.State.Running}}' "$SUPERVISORE" 2>/dev/null)" = true ] && return 0
                sleep 3
            done
            echo "nessun battito dopo il riavvio: il supervisore $SUPERVISORE non gira" >&2
            docker logs --tail 5 "$SUPERVISORE" >&2 || true
            return 1
        fi
        sleep 3
    done
    echo "l'agente non risponde su 127.0.0.1:3180" >&2
    return 1
}

case "${1:-}" in
    salva)
        # tutto tranne i dati che vivono solo sul server
        tar -czf "$COPIA" --exclude=./.env --exclude=./agent/fonti.json \
            --exclude=./telefono --exclude=./stato --exclude=./ui-dati --exclude=./.versione \
            --exclude='./apk/*.apk' . 2>/dev/null || true
        echo "copia: $COPIA"
        ;;
    avvia)
        sha="${2:-sconosciuto}"
        credenziali_da_stdin
        for f in .env agent/fonti.json; do
            [ -f "$f" ] || { echo "$f mancante in $DIR: la prima installazione va fatta a mano (vedi GUIDA.md)" >&2; exit 1; }
        done
        echo "aggiornamento a $sha"
        if avvia && sano; then
            echo "$sha" > .versione
            ui_fallita=0
            avvia_ui || ui_fallita=1
            avvia_schermo || ui_fallita=1
            docker image prune -f >/dev/null
            docker ps --filter name=oe-copilot- --format '{{.Names}}: {{.Status}}'
            echo "ok: $sha in esecuzione"
            [ "$ui_fallita" = 0 ] || { echo "l'interfaccia web non è partita (agente e telefono a posto)" >&2; exit 1; }
            exit 0
        fi
        echo "aggiornamento a $sha fallito: si torna alla versione di prima" >&2
        docker compose logs --tail 40 agent >&2 || true
        if [ -f "$COPIA" ]; then
            tar -xzf "$COPIA"
            avvia && sano && echo "tornato a: $(cat .versione 2>/dev/null || echo versione precedente)" >&2
        fi
        exit 1
        ;;
    *)
        echo "uso: $0 salva | avvia <sha>" >&2; exit 2
        ;;
esac
