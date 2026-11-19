#!/bin/bash
# EverID davanti all'interfaccia (README.md 4.7): Caddy e il gateway in Rust veri, con un outpost di Authentik finto. Prova che
# l'interfaccia chiede l'accesso a EverID, che meet, /mcp, /condividi/ e /salute non passano da EverID, e che un X-Authentik-Email
# mandato dal client non arriva mai al gateway, nemmeno sulle strade libere. Serve Docker; gira anche in Git Bash su Windows.
#   bash calendar/scripts/everid.sh      (lo lancia scripts/check.sh calendar --docker)
set -euo pipefail
docker() { MSYS_NO_PATHCONV=1 command docker "$@"; }
srv=$(cd "$(dirname "$0")/../server" && { pwd -W 2>/dev/null || pwd; })
rete=oe-everid-prova
email=prova@example.com
pw=$(openssl rand -hex 16 | tr -d '\r')

docker build -q -t oe-copilot-caddy "$srv/caddy-ui" >/dev/null
docker build -q -t oe-copilot-gateway-rs -f "$srv/agente-rs/gateway.Dockerfile" --build-context web="$srv/web-ts" "$srv/agente-rs" >/dev/null

docker network rm "$rete" >/dev/null 2>&1 || true
docker network create "$rete" >/dev/null
ids=()
trap 'rc=$?; [ $rc = 0 ] || for i in "${ids[@]}"; do docker logs --tail 30 "$i"; done; docker rm -f "${ids[@]}" >/dev/null 2>&1; docker network rm "$rete" >/dev/null' EXIT

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
ids+=("$(docker run -d --network "$rete" --network-alias outpost --entrypoint sh caddy:2.11.4-alpine \
  -c 'printf "%s" "$CF" > /tmp/Caddyfile && exec caddy run --config /tmp/Caddyfile --adapter caddyfile' -e CF="$outpost")")
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
pronto() { for _ in $(seq 1 60); do [ "$(codice "${ui[@]}" $C/salute)" = 200 ] && [ "$(codice -k $M/)" = 200 ] && return 0; sleep 1; done; return 1; }
pronto

echo "== interfaccia: senza EverID rimanda a EverID, con EverID entra"
test "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' "${ui[@]}" $C/)" = "302 https://id.example/login"
test "$(codice "${ui[@]}" -X POST -H 'Content-Type: application/json' -d '[]' $C/api/get_config)" = 302
curl -fsS "${ui[@]}" -b everid=ok $C/ | grep -q '"esci":"/outpost.goauthentik.io/sign_out"'
test "$(codice "${ui[@]}" -b everid=ok -X POST -H 'Content-Type: application/json' -d '[]' $C/api/get_config)" = 200
test "$(curl -s "${ui[@]}" $C/outpost.goauthentik.io/sign_out)" = outpost

echo "== strade libere: nessun rimando a EverID"
test "$(codice "${ui[@]}" $C/salute)" = 200
test "$(codice "${ui[@]}" -X POST -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' $C/mcp)" = 401
test "$(codice "${ui[@]}" -X POST -H 'Content-Type: application/json' -H 'Authorization: Bearer mcp-di-prova' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' $C/mcp)" = 200
test "$(codice "${ui[@]}" "$C/condividi/$(openssl rand -hex 22 | cut -c1-43).ics")" = 404
test "$(codice "${meet[@]}" $M/)" = 200
curl -fsS "${meet[@]}" $M/ | grep -q '<title>Prenota un meeting'
test "$(codice "${meet[@]}" $M/login)" = 302

echo "== intestazione inventata dal client: Caddy la toglie ovunque"
finto=(-H "X-Authentik-Email: $email")
# sull'interfaccia: senza il cookie di EverID resta il rimando, l'intestazione del client non basta
test "$(codice "${ui[@]}" "${finto[@]}" $C/)" = 302
# su meet, che non passa da EverID, la pagina resta quella pubblica e il resto rimanda alla radice
test "$(codice "${meet[@]}" "${finto[@]}" $M/disponibilita)" = 200
test "$(codice "${meet[@]}" "${finto[@]}" -X POST -H 'Content-Type: application/json' -d '[]' $M/api/get_config)" = 302
# /mcp e /salute non cambiano con l'intestazione
test "$(codice "${ui[@]}" "${finto[@]}" -X POST -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' $C/mcp)" = 401
test "$(codice "${ui[@]}" "${finto[@]}" $C/salute)" = 200
test "$(codice "${ui[@]}" "${finto[@]}" $C/condividi/x.ics)" = 404
# la prova vera della pulizia: Caddy senza EverID (EVERID_UPSTREAM vuoto) e il gateway che si fida di lui. Ogni richiesta arriva
# al gateway da Caddy: un X-Authentik-Email che passasse farebbe entrare. Deve restare il rimando all'accesso del gateway
docker rm -f "$cd_id" >/dev/null
cd_id=$(caddy ""); ids+=("$cd_id")
pronto
test "$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' "${ui[@]}" "${finto[@]}" $C/)" = "307 http://ui.test:8080/login?next=%2F"
test "$(codice "${ui[@]}" "${finto[@]}" -H 'x-authentik-email: '"$email" -X POST -H 'Content-Type: application/json' -d '[]' $C/api/get_config)" = 401
test "$(codice "${ui[@]}" "${finto[@]}" $C/logos/personale.png)" = 302

echo "== gateway: da un container che non è Caddy l'intestazione non vale"
test "$(docker run --rm --network "$rete" curlimages/curl:8.17.0 -s -o /dev/null -w '%{http_code}' -H "X-Authentik-Email: $email" \
  -X POST -H 'Content-Type: application/json' -d '[]' http://gateway:3000/api/get_config)" = 401
echo "== EverID: tutto a posto"
