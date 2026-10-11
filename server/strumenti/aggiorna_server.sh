#!/bin/bash
# Oe Copilot - aggiornamento del server, lanciato dalla GitHub Action via SSH.
#   aggiorna_server.sh salva        mette da parte il codice attuale (per tornare indietro)
#   aggiorna_server.sh avvia <sha>  ricostruisce e riavvia; se l'agente non risponde torna alla versione salvata
# Il codice arriva con rsync dalla Action; .env, agent/fonti.json, telefono/, stato/ e
# ui-dati/ restano sul server e non vengono toccati.
set -euo pipefail
DIR="${OE_DIR:-/opt/oe-copilot}"
COPIA=/var/tmp/oe-copilot-prima.tgz
SERVIZIO=oe-copilot-sincronizza
T=oe-copilot-telefono
cd "$DIR"

avvia() {
    lsmod | grep -q '^binder_linux' \
        || { echo "modulo binder_linux non caricato (vedi GUIDA.md, 4.1)" >&2; return 1; }
    # Il telefono si ricrea solo se la sua configurazione è cambiata: un riavvio
    # inutile fermerebbe le sincronizzazioni per un paio di minuti.
    docker compose up -d --build telefono
    # L'agente monta la cartella del calendario del telefono: deve esistere prima,
    # altrimenti Docker la crea vuota e di root al posto di Android.
    for _ in $(seq 1 60); do
        [ "$(docker exec "$T" getprop sys.boot_completed 2>/dev/null)" = 1 ] && break
        sleep 5
    done
    [ "$(docker exec "$T" getprop sys.boot_completed 2>/dev/null)" = 1 ] \
        || { echo "il telefono non ha finito di avviarsi" >&2; return 1; }
    # impostazioni del telefono e negozio delle app: stanno nello script, non a mano
    "$DIR/strumenti/prepara_telefono.sh"
    # I file .py sono montati uno per uno: rsync li sostituisce con file nuovi e il
    # container continuerebbe a leggere i vecchi. Per questo l'agente si ricrea sempre.
    docker compose up -d --build --force-recreate agent
    sudo -n systemctl daemon-reload
    riavvio=$(date +%s)
    sudo -n systemctl restart "$SERVIZIO"
}

# Interfaccia web (facoltativa, GUIDA.md 4.7): il servizio ui, il gateway che chiede
# l'accesso e il Caddy che le mette HTTPS davanti. Parte solo se in .env c'è UI_HOST.
# Gira dopo che agente e battito sono a posto e non fa tornare indietro niente: se la web
# non parte, la Action diventa rossa ma l'agente e il telefono restano come sono. Il Caddy
# nuovo si ricrea solo se il gateway è sano (depends_on): se non lo è resta quello di prima.
avvia_ui() {
    grep -q '^UI_HOST=.' .env || { echo "interfaccia web non configurata (UI_HOST in .env): salto"; return 0; }
    [ -d ui-dati ] || { echo "manca $DIR/ui-dati (vedi GUIDA.md, 4.7)" >&2; return 1; }
    for v in ADMIN_EMAIL ADMIN_PASSWORD BETTER_AUTH_SECRET; do
        grep -q "^$v=." .env || { echo "manca $v in $DIR/.env (vedi GUIDA.md, 4.7): l'interfaccia web resta com'è" >&2; return 1; }
    done
    docker compose up -d --build ui gateway caddy-ui
    for servizio in oe-copilot-ui oe-copilot-gateway; do
        stato=""
        for _ in $(seq 1 30); do
            stato="$(docker inspect -f '{{.State.Health.Status}}' "$servizio" 2>/dev/null)"
            [ "$stato" = healthy ] && break
            sleep 2
        done
        [ "$stato" = healthy ] || { echo "il servizio $servizio non risponde" >&2; docker logs --tail 40 "$servizio" >&2 || true; return 1; }
    done
    # il cancello deve chiudere davvero: senza sessione la pagina rimanda a /login e l'API risponde 401
    [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 http://127.0.0.1:3190/index.html)" = 302 ] \
        && [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 -X POST -H 'Content-Type: application/json' -d '[]' http://127.0.0.1:3190/api/get_config)" = 401 ] \
        || { echo "il gateway non chiude l'accesso come dovrebbe" >&2; return 1; }
}

sano() {
    chiave="$(grep '^API_KEY=' .env | cut -d= -f2)"
    for _ in $(seq 1 20); do
        if curl -fsS --max-time 5 -H "Authorization: Bearer $chiave" \
                http://127.0.0.1:3180/api/salute >/dev/null 2>&1; then
            # il servizio scrive il battito a ogni giro: ne serve uno scritto dopo il
            # riavvio, quello di prima non dice niente
            for _ in $(seq 1 30); do
                [ "$(stat -c %Y stato/battito.json 2>/dev/null || echo 0)" -ge "$riavvio" ] \
                    && systemctl is-active -q "$SERVIZIO" && return 0
                sleep 3
            done
            echo "nessun battito dopo il riavvio: il servizio $SERVIZIO non gira" >&2
            systemctl status "$SERVIZIO" --no-pager -n 5 >&2 || true
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
        for f in .env agent/fonti.json; do
            [ -f "$f" ] || { echo "$f mancante in $DIR: la prima installazione va fatta a mano (vedi GUIDA.md)" >&2; exit 1; }
        done
        echo "aggiornamento a $sha"
        if avvia && sano; then
            echo "$sha" > .versione
            ui_fallita=0
            avvia_ui || ui_fallita=1
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
