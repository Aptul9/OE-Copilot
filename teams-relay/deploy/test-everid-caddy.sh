#!/usr/bin/env bash
# Proxy routes reach the web app without redirects to the identity service, and client identity headers are stripped.
# The web app itself enforces session, Relay and MCP authorization.
set -euo pipefail

IMAGE=${CADDY_IMAGE:-caddy:2.11.8-alpine}
HERE=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
NET=everid-test-$$
PORT=${TEST_PORT:-18080}
if command -v cygpath >/dev/null 2>&1; then
  docker() { MSYS_NO_PATHCONV=1 command docker "$@"; }
  MOUNT_WORK=$(cygpath -m "$WORK")
  MOUNT_CADDY=$(cygpath -m "$HERE/caddy")
else
  MOUNT_WORK=$WORK
  MOUNT_CADDY=$HERE/caddy
fi
SPOOF="spoof""@""example.com"
cleanup() {
  docker rm -f "$NET-webapp" "$NET-caddy" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

cat > "$WORK/webapp.caddy" <<'EOF'
:8090 {
	respond "webapp {path} email=[{header.X-Authentik-Email}]" 200
}
EOF
docker network create "$NET" >/dev/null
docker run -d --name "$NET-webapp" --network "$NET" --network-alias webapp -v "$MOUNT_WORK/webapp.caddy:/etc/caddy/Caddyfile:ro" "$IMAGE" >/dev/null

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
    -v "$MOUNT_CADDY:/etc/caddy:ro" "$IMAGE" >/dev/null
  for _ in $(seq 1 30); do
    curl -s --max-time 15 -o /dev/null -H 'Host: teams.test' "http://127.0.0.1:$PORT/api/health" && return
    sleep 1
  done
  docker logs "$NET-caddy" >&2
  exit 1
}
URL=http://127.0.0.1:$PORT

start on
check "on: page reaches the web app" "200 " "$(get "$URL/")"
check "on: login reaches the web app" "200 " "$(get "$URL/login")"
check "on: spoofed identity stripped" "200 " "$(get -H "X-Authentik-Email: $SPOOF" -H 'X-Authentik-Groups: app-teams' "$URL/settings")"
check "on: web app sees no identity" "webapp /settings email=[]" "$(body)"
for path in /mcp /api/relay/status /sw.js /api/push/subscribe /outpost.goauthentik.io/sign_out; do
  check "on: $path without forward auth" "200 " "$(get -H "X-Authentik-Email: $SPOOF" "$URL$path")"
  check "on: $path spoofed header dropped" "webapp $path email=[]" "$(body)"
done

start off
check "off: page reaches the web app" "200 " "$(get "$URL/")"
check "off: spoofed header dropped" "webapp / email=[]" "$(get -H "X-Authentik-Email: $SPOOF" "$URL/" >/dev/null; body)"

exit $failed
