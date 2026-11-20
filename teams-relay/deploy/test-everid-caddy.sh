#!/usr/bin/env bash
# Behaviour of caddy/Caddyfile with EverID, against stand-ins of the outpost and of the web app (Docker needed):
# - EVERID_MODE=on: a page without an EverID session goes to the EverID sign-in; with one it reaches the web app
#   with the outpost's X-Authentik-Email, never the one sent by the client
# - the API, /mcp, the service worker and the push endpoints reach the web app without forward auth, and without
#   X-Authentik-* headers sent by the client
# - EVERID_MODE=off: the pages reach the web app as before
set -euo pipefail

IMAGE=${CADDY_IMAGE:-caddy:2.11.8-alpine}
HERE=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
NET=everid-test-$$
PORT=${TEST_PORT:-18080}
SPOOF="spoof""@""example.com"
SIGNED="signed""@""example.com"
cleanup() {
  docker rm -f "$NET-webapp" "$NET-outpost" "$NET-caddy" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

cat > "$WORK/webapp.caddy" <<'EOF'
:8090 {
	respond "webapp {path} email=[{header.X-Authentik-Email}]" 200
}
EOF
cat > "$WORK/outpost.caddy" <<EOF
:9000 {
	@signed {
		path /outpost.goauthentik.io/auth/caddy
		header Cookie *ak_session=1*
	}
	handle @signed {
		header X-Authentik-Email $SIGNED
		header X-Authentik-Groups app-teams
		respond 200
	}
	handle /outpost.goauthentik.io/auth/caddy {
		redir https://id.example/flow 302
	}
	respond 404
}
EOF

docker network create "$NET" >/dev/null
docker run -d --name "$NET-webapp" --network "$NET" --network-alias webapp -v "$WORK/webapp.caddy:/etc/caddy/Caddyfile:ro" "$IMAGE" >/dev/null
docker run -d --name "$NET-outpost" --network "$NET" --network-alias outpost -v "$WORK/outpost.caddy:/etc/caddy/Caddyfile:ro" "$IMAGE" >/dev/null

failed=0
check() {
  local name=$1 want=$2 got=$3
  if [[ "$got" == "$want" ]]; then echo "PASS $name"; else echo "FAIL $name: got [$got], want [$want]"; failed=1; fi
}
get() { curl -s -o "$WORK/body" -w '%{http_code} %{redirect_url}' -H 'Host: teams.test' "$@"; }
body() { cat "$WORK/body"; }

start() {
  docker rm -f "$NET-caddy" >/dev/null 2>&1 || true
  docker run -d --name "$NET-caddy" --network "$NET" -p "127.0.0.1:$PORT:80" \
    -e DOMAIN=http://teams.test -e EVERID_MODE="$1" -e EVERID_OUTPOST=http://outpost:9000 \
    -v "$HERE/caddy:/etc/caddy:ro" "$IMAGE" >/dev/null
  for _ in $(seq 1 30); do
    curl -s -o /dev/null -H 'Host: teams.test' "http://127.0.0.1:$PORT/api/health" && return
    sleep 1
  done
  docker logs "$NET-caddy" >&2
  exit 1
}
URL=http://127.0.0.1:$PORT

start on
check "on: page without EverID session goes to sign-in" "302 https://id.example/flow" "$(get "$URL/")"
check "on: admin page without EverID session goes to sign-in" "302 https://id.example/flow" "$(get "$URL/admin")"
check "on: page with EverID session reaches the web app" "200 " "$(get -H 'Cookie: ak_session=1' -H "X-Authentik-Email: $SPOOF" "$URL/")"
check "on: web app gets the outpost's email, not the client's" "webapp / email=[$SIGNED]" "$(body)"
check "on: spoofed header alone does not pass the gate" "302 https://id.example/flow" "$(get -H "X-Authentik-Email: $SPOOF" -H 'X-Authentik-Groups: app-teams' "$URL/settings")"
for path in /mcp /api/relay/status /sw.js /api/push/subscribe; do
  check "on: $path without forward auth" "200 " "$(get -H "X-Authentik-Email: $SPOOF" "$URL$path")"
  check "on: $path spoofed header dropped" "webapp $path email=[]" "$(body)"
done

start off
check "off: page reaches the web app" "200 " "$(get "$URL/")"
check "off: spoofed header dropped" "webapp / email=[]" "$(get -H "X-Authentik-Email: $SPOOF" "$URL/" >/dev/null; body)"

exit $failed
