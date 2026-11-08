#!/bin/bash
# Checks to run before a push to main, which deploys: the repository has no CI. The steps are those the CI of each
# module ran before the merge.
#   scripts/check.sh <calendar|teams-relay|mcp|all>... [--docker]
# --docker adds compose files, Caddy and the images (Docker needed); for calendar also the agent and gateway images
# end to end (calendar/scripts/integrazione.sh). Runs in Git Bash on Windows too.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)

usage() { echo "usage: $0 <calendar|teams-relay|mcp|all>... [--docker]" >&2; exit 2; }
docker=0
mods=()
for a in "$@"; do
  case $a in
    --docker) docker=1 ;;
    all) mods+=(calendar teams-relay mcp) ;;
    calendar|teams-relay|mcp) mods+=("$a") ;;
    *) usage ;;
  esac
done
[ ${#mods[@]} -gt 0 ] || usage

step() { printf '\n== %s\n' "$*"; }
# Git Bash: paths inside docker arguments belong to the container and must not become Windows paths
docker() { MSYS_NO_PATHCONV=1 command docker "$@"; }
# no install scripts: better-sqlite3 would run node-gyp, and Windows has no compiler (the package ships its binaries)
install() { npm ci --ignore-scripts --no-audit --no-fund; }

check_calendar() {
  cd "$root/calendar"
  step "calendar: Python and shell syntax"
  python3 -m py_compile server/strumenti/*.py
  for f in server/strumenti/*.sh; do bash -n "$f"; done
  step "calendar: gateway"
  (cd server/web-ts && install && npm run typecheck && npm test && npm run build)
  step "calendar: agent"
  (cd server/agente-ts && install && npm run typecheck && npm test)
  [ "$docker" = 1 ] || return 0
  step "calendar: compose"
  # no service sits behind a profile: the list is what `docker compose up` starts
  (cd server && export API_KEY=prova && docker compose config -q \
    && test "$(docker compose config --services | sort | tr '\n' ' ')" = "agent caddy-ui gateway ")
  step "calendar: images end to end"
  bash scripts/integrazione.sh
}

check_teams_relay() {
  cd "$root/teams-relay/app"
  step "teams-relay: app"
  install
  npm run lint
  npm run typecheck
  npm test
  # the bundles no image carries: the local relay and the fleet
  node scripts/bundles.mjs relay cmdapi fleet fleet-agent
  node --check public/sw.js
  node dist/relay.cjs --check
  cd "$root/teams-relay/mobile"
  step "teams-relay: Android app (page tests, shared Rust)"
  install
  npm test
  # with Docker in a Linux Rust image, as on the old runner (a Windows GNU toolchain has no C compiler for ring)
  if [ "$docker" = 1 ]; then
    docker run --rm -v "$(pwd -W 2>/dev/null || pwd)/core:/core:ro" -w /core -e CARGO_TARGET_DIR=/tmp/target rust:1 cargo test
  elif command -v cargo >/dev/null; then
    cargo test --manifest-path core/Cargo.toml
  else
    echo "no cargo: core/ not tested"
  fi
  cd "$root/teams-relay"
  step "teams-relay: shell syntax"
  bash -n deploy/remote-deploy.sh
  bash -n deploy/render-env.sh
  [ "$docker" = 1 ] || return 0
  export DOMAIN=example.com
  step "teams-relay: compose files"
  docker compose config -q
  docker compose --env-file compose.local.env -f docker-compose.yml -f compose.local.yml config -q
  step "teams-relay: Caddyfile"
  docker compose run --rm --no-deps caddy caddy validate --config /etc/caddy/Caddyfile
  step "teams-relay: images"
  # the browsers image runs node agent.cjs --check and node supervisor.cjs --check while it builds
  docker compose build -q webapp browsers
}

check_mcp() {
  cd "$root/mcp"
  step "mcp"
  install
  npm run typecheck
  npm test
  [ "$docker" = 1 ] || return 0
  step "mcp: image"
  docker build -q -t communication-mcp-check .
}

for m in "${mods[@]}"; do
  case $m in
    calendar) check_calendar ;;
    teams-relay) check_teams_relay ;;
    mcp) check_mcp ;;
  esac
done
step "all checks passed: ${mods[*]}"
