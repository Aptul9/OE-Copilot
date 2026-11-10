#!/bin/bash
# Checks the on-demand desktop in a throwaway container of the browsers image, on a Docker host:
#   scripts/lazy-desktop.sh <image>      image: docker build --target browsers -t <image> app
# It starts one container, drives the real supervisor through its control socket (sign-in of account 99, which starts a
# browser without touching any account), removes it, then checks DESKTOP_ON_DEMAND=0 in a second one. About 5 minutes: the
# idle stop is waited for.
set -u
image=${1:?usage: $0 <image>}
name=lazy-desktop-check
fail=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fail=1; }
now() { date +%s.%N; }
since() { echo "scale=1; $(now) - $1" | bc; }
svc() { docker exec "$name" s6-rc -a list 2>&1 | grep -E '^svc-' | sort | tr '\n' ' '; }
mem() { docker stats --no-stream --format '{{.MemUsage}}' "$name"; }
ctl() {
  docker exec "$name" node -e "const r=require('http').request({socketPath:'/root/run/control.sock',method:process.argv[1],path:process.argv[2]},x=>{x.resume();x.on('end',()=>console.log(x.statusCode))});r.on('error',e=>{console.log(e.message);process.exit(1)});r.end()" "$1" "$2"
}
box() {
  docker rm -f "$name" >/dev/null 2>&1
  docker run -d --name "$name" --security-opt seccomp=unconfined --shm-size=1g --tmpfs /profiles:rw,uid=1000,gid=1000,size=512m \
    -e PUID=1000 -e PGID=1000 -e TZ=Europe/Rome -e PIXELFLUX_WAYLAND=true -e CUSTOM_USER=check -e PASSWORD=check \
    -e SUBFOLDER=/desktop/ -e SELKIES_ENCODER=jpeg -e NATIVE_CALLS=0 -e LIGHT_HOST=1 -e DOMAIN=check.invalid \
    -e DESKTOP_ON_DEMAND="$1" -e DESKTOP_IDLE_MIN=1 "$image" >/dev/null
  local t0
  t0=$(now)
  for _ in $(seq 1 120); do docker exec "$name" node /app/supervisor.cjs status >/dev/null 2>&1 && break; sleep 0.5; done
  docker exec "$name" node /app/supervisor.cjs status >/dev/null 2>&1 && ok "supervisor answers after $(since "$t0") s" || bad "supervisor does not answer"
  sleep 10
}
trap 'docker rm -f "$name" >/dev/null 2>&1' EXIT

box 1
up=$(svc)
[ "$up" = "svc-cron svc-docker svc-teamsrelay svc-watchdog " ] && ok "boot: only $up" || bad "boot services: $up"
echo "     container memory, supervisor only: $(mem)"

t=$(now)
code=$(ctl POST /accounts/99/signin)
[ "$code" = 204 ] && ok "sign-in of account 99 answered 204 after $(since "$t") s" || bad "sign-in answered $code"
up=$(svc)
case "$up" in *svc-de*svc-xorg*) ok "desktop services up";; *) bad "desktop services: $up";; esac
c=$(docker exec "$name" sh -c 'curl -s -o /dev/null -w %{http_code} --max-time 3 http://127.0.0.1:3000/desktop/')
case "$c" in 200|401|301|302) ok "desktop answers (HTTP $c)";; *) bad "desktop answers $c";; esac
docker exec "$name" pgrep -f '^/usr/lib/chromium/chromium' >/dev/null && ok "Chromium runs" || bad "no Chromium"
echo "     container memory, desktop and browser up: $(mem)"

[ "$(ctl POST /accounts/99/stop)" = 204 ] && ok "account 99 stopped" || bad "stop failed"
t=$(now)
for _ in $(seq 1 240); do case "$(svc)" in *svc-de*) sleep 1;; *) break;; esac; done
case "$(svc)" in *svc-de*) bad "desktop still up after $(since "$t") s";; *) ok "desktop stopped $(since "$t") s after the browser";; esac
[ "$(docker exec "$name" node /app/supervisor.cjs status >/dev/null 2>&1; echo $?)" = 0 ] && ok "supervisor still answers" || bad "supervisor does not answer"
echo "     container memory, desktop down: $(mem)"

t=$(now)
code=$(ctl POST /accounts/99/signin)
[ "$code" = 204 ] && ok "second sign-in answered 204 after $(since "$t") s" || bad "second sign-in answered $code"
docker exec "$name" pgrep -f '^/usr/lib/chromium/chromium' >/dev/null && ok "Chromium runs again" || bad "no Chromium after the second sign-in"
ctl POST /accounts/99/stop >/dev/null
ctl POST /accounts/99/wipe >/dev/null

# the rollback switch: the desktop comes up with the supervisor, as before
box 0
for _ in $(seq 1 60); do case "$(svc)" in *svc-de*svc-xorg*) break;; *) sleep 1;; esac; done
case "$(svc)" in *svc-de*svc-xorg*) ok "DESKTOP_ON_DEMAND=0: desktop up at boot";; *) bad "DESKTOP_ON_DEMAND=0, services: $(svc)";; esac
# what a sign-in costs with the desktop already up, to compare with the first one above
t=$(now)
code=$(ctl POST /accounts/99/signin)
[ "$code" = 204 ] && ok "DESKTOP_ON_DEMAND=0: sign-in answered 204 after $(since "$t") s" || bad "DESKTOP_ON_DEMAND=0: sign-in answered $code"
ctl POST /accounts/99/stop >/dev/null
ctl POST /accounts/99/wipe >/dev/null
[ "$fail" = 0 ] && echo "all checks passed" || echo "CHECKS FAILED"
exit "$fail"
