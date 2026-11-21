#!/bin/bash
# Test proxy: local login, public routes, bearer MCP and spoofed identity headers.
# The Authentik flow itself is tested with the HTTP fixture in gateway/tests/servizio.rs.
#   bash calendar/scripts/everid.sh      (lo lancia scripts/check.sh calendar --docker)
set -euo pipefail
# grep -q legge tutto l'ingresso: con pipefail, `curl ... | grep -q` fermato alla prima riga uguale fa morire curl di SIGPIPE (141)
grep() { case "${1:-}" in -q) shift; command grep "$@" >/dev/null ;; -qi) shift; command grep -i "$@" >/dev/null ;; *) command grep "$@" ;; esac; }
docker() { MSYS_NO_PATHCONV=1 command docker "$@"; }
srv=$(cd "$(dirname "$0")/../server" && { pwd -W 2>/dev/null || pwd; })
rete="oe-everid-prova-$$"
email=prova@example.com
pw=$(openssl rand -hex 16 | tr -d '\r')

docker build -q -t oe-copilot-caddy "$srv/caddy-ui" >/dev/null
docker build -q -t oe-copilot-gateway-rs -f "$srv/agente-rs/gateway.Dockerfile" --build-context web="$srv/web-ts" "$srv/agente-rs" >/dev/null

docker network create "$rete" >/dev/null
ids=()
trap 'echo "everid.sh: riga $LINENO: $BASH_COMMAND" >&2' ERR
trap 'rc=$?; [ $rc = 0 ] || for i in "${ids[@]}"; do docker logs --tail 30 "$i" 2>/dev/null || true; done; docker rm -f "${ids[@]}" >/dev/null 2>&1; docker network rm "$rete" >/dev/null' EXIT

# l'outpost finto: con il cookie everid=ok risponde 200 e l'email, senza rimanda alla pagina di EverID
outpost='
:9000 {
	@entrato {
		path /outpost.goauthentik.io/auth/caddy
		header Cookie *everid=ok*
	}
	handle @entrato {
		header X-Authentik-Email '"$email"'
		respond 200
	}
	handle /outpost.goauthentik.io/auth/caddy {
		redir https://id.example/login 302
	}
	handle /outpost.goauthentik.io/* {
		respond "outpost" 200
	}
}'
ids+=("$(docker run -d --network "$rete" --network-alias outpost -e CF="$outpost" --entrypoint sh caddy:2.11.4-alpine \
  -c 'printf "%s" "$CF" > /tmp/Caddyfile && exec caddy run --config /tmp/Caddyfile --adapter caddyfile')")
ids+=("$(docker run -d --network "$rete" --network-alias gateway --read-only --tmpfs /tmp -e GATEWAY_DB=/tmp/gw.db -e OE_DATI=/tmp/dati \
  -e AGENTE_URL=http://agent:8180 -e AGENTE_CHIAVE=prova -e MCP_TOKEN=mcp-di-prova -e APP_URL=http://ui.test:8080 \
  -e BETTER_AUTH_SECRET="$(openssl rand -hex 32)" -e ADMIN_EMAIL="$email" -e ADMIN_PASSWORD="$pw" \
  -e EVERID=on -e EVERID_PROXY=caddy-ui oe-copilot-gateway-rs)")
# MEET_HOST è un nome senza schema né porta (il confronto con {host}): localhost, con il certificato interno di Caddy
caddy() { docker run -d --network "$rete" --network-alias caddy-ui -p 8088:8080 -p 8443:443 \
  -e UI_HOST=http://ui.test:8080 -e MEET_HOST=localhost -e EVERID_UPSTREAM="$1" oe-copilot-caddy; }
cd_id=$(caddy outpost:9000); ids+=("$cd_id")

C=http://127.0.0.1:8088
M=https://localhost:8443
ui=(-H 'Host: ui.test:8080')
meet=(-k)
codice() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
# confronto che dice riga, valore avuto e valore atteso quando non torna
uguale() { [ "$1" = "$2" ] || { echo "everid.sh riga ${BASH_LINENO[0]}: avuto [$1], atteso [$2]" >&2; return 1; }; }
pronto() { for _ in $(seq 1 60); do [ "$(codice "${ui[@]}" $C/salute)" = 200 ] && [ "$(codice -k $M/)" = 200 ] && return 0; sleep 1; done; return 1; }
pronto

echo "== interface: local login without external redirects"
uguale "$(codice "${ui[@]}" $C/)" 307
uguale "$(codice "${ui[@]}" $C/login)" 200
uguale "$(codice "${ui[@]}" -X POST -H 'Content-Type: application/json' -d '[]' $C/api/get_config)" 401
uguale "$(codice "${ui[@]}" $C/outpost.goauthentik.io/sign_out)" 302

echo "== strade libere: nessun rimando a EverID"
uguale "$(codice "${ui[@]}" $C/salute)" 200
uguale "$(codice "${ui[@]}" -X POST -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' $C/mcp)" 401
uguale "$(codice "${ui[@]}" -X POST -H 'Content-Type: application/json' -H 'Authorization: Bearer mcp-di-prova' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' $C/mcp)" 200
uguale "$(codice "${ui[@]}" "$C/condividi/$(openssl rand -hex 22 | cut -c1-43).ics")" 404
uguale "$(codice "${meet[@]}" $M/)" 200
curl -fsS "${meet[@]}" $M/ | grep -q '<title>Prenota un meeting'
uguale "$(codice "${meet[@]}" $M/login)" 302

echo "== intestazione inventata dal client: Caddy la toglie ovunque"
finto=(-H "X-Authentik-Email: $email")
# sull'interfaccia: senza il cookie di EverID resta il rimando, l'intestazione del client non basta
uguale "$(codice "${ui[@]}" "${finto[@]}" $C/)" 307
# su meet, che non passa da EverID, la pagina resta quella pubblica e il resto rimanda alla radice
uguale "$(codice "${meet[@]}" "${finto[@]}" $M/disponibilita)" 200
uguale "$(codice "${meet[@]}" "${finto[@]}" -X POST -H 'Content-Type: application/json' -d '[]' $M/api/get_config)" 302
# /mcp e /salute non cambiano con l'intestazione
uguale "$(codice "${ui[@]}" "${finto[@]}" -X POST -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' $C/mcp)" 401
uguale "$(codice "${ui[@]}" "${finto[@]}" $C/salute)" 200
uguale "$(codice "${ui[@]}" "${finto[@]}" $C/condividi/x.ics)" 404
# la prova vera della pulizia: Caddy senza EverID (EVERID_UPSTREAM vuoto) e il gateway che si fida di lui. Ogni richiesta arriva
# al gateway da Caddy: un X-Authentik-Email che passasse farebbe entrare. Deve restare il rimando all'accesso del gateway
docker rm -f "$cd_id" >/dev/null
cd_id=$(caddy ""); ids+=("$cd_id")
pronto
uguale "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' "${ui[@]}" "${finto[@]}" $C/)" "307 $C/login?next=%2F"
uguale "$(codice "${ui[@]}" "${finto[@]}" -H 'x-authentik-email: '"$email" -X POST -H 'Content-Type: application/json' -d '[]' $C/api/get_config)" 401
uguale "$(codice "${ui[@]}" "${finto[@]}" $C/logos/personale.png)" 302

echo "== gateway: da un container che non è Caddy l'intestazione non vale"
uguale "$(docker run --rm --network "$rete" curlimages/curl:8.17.0 -s -o /dev/null -w '%{http_code}' -H "X-Authentik-Email: $email" \
  -X POST -H 'Content-Type: application/json' -d '[]' http://gateway:3000/api/get_config)" 401
echo "== EverID: tutto a posto"
