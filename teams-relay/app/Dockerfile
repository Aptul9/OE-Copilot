FROM node:26-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
# better-sqlite3 ships prebuilt binaries and esbuild its binary in an optional package: no install scripts,
# no compiler
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY . .
ENV NEXT_TELEMETRY_DISABLED=1
# the pages of the web app exported as static files (out/, served by webapp-rs), agent (dist/agent.cjs), light agent
# (dist/light.cjs) and supervisor (dist/supervisor.cjs); the local relay and the fleet bundles are no image's
RUN TR_EXPORT=1 npx next build && node scripts/bundles.mjs agent light supervisor
# What the agent loads at runtime besides its bundle. The standalone output traces only the web app, so these
# are copied explicitly: playwright-core whole (no dependencies), better-sqlite3 without its sources and with
# the binary of this architecture only (the image is built on the machine that runs it).
RUN mkdir -p /agent/node_modules/better-sqlite3/prebuilds \
 && cp -r node_modules/playwright-core /agent/node_modules/ \
 && cp -r node_modules/better-sqlite3/package.json node_modules/better-sqlite3/lib /agent/node_modules/better-sqlite3/ \
 && cp "node_modules/better-sqlite3/prebuilds/linux-$(node -p process.arch).node" /agent/node_modules/better-sqlite3/prebuilds/ \
 && cp dist/agent.cjs dist/agent.cjs.map dist/light.cjs dist/light.cjs.map dist/supervisor.cjs /agent/

# Native calling worker and its exact reviewed vendor artifacts. No package install scripts.
RUN cp -r node_modules/@roamhq /agent/node_modules/ \
 && cp -r node_modules/opusscript node_modules/ws node_modules/domexception node_modules/webidl-conversions /agent/node_modules/ \
 && cp scripts/calling-live-host.mjs scripts/calling-host-helpers.mjs scripts/calling-audio-bridge.mjs scripts/trouter-live-probe.mjs scripts/trouter-offline-probe.mjs /agent/ \
 && printf '{"private":true}' > /agent/package.json \
 && node scripts/fetch-calling.mjs

# Every Teams account: Chromium and the remote desktop of linuxserver, the agents, and the supervisor that runs
# them (s6 service svc-teamsrelay). The desktop no longer opens a browser by itself.
FROM lscr.io/linuxserver/chromium:latest@sha256:59d1bfa98a6ab759fce078e5a714de3920f4c618b8939a0df6e1dec7d25c7aba AS browsers
# wlrctl brings the window of an account to the front. abc, the user of the browsers, loses its passwordless
# sudo: a browser taken over by a web page must not reach root and, with it, data/ and vapid/.
RUN apt-get update \
 && apt-get install -y --no-install-recommends wlrctl \
 && rm -rf /var/lib/apt/lists/* \
 && gpasswd -d abc sudo
COPY --from=build /usr/local/bin/node /usr/local/bin/node
COPY --from=build /agent /app
COPY docker/browsers/ /
# The remote desktop starts when a browser needs it (src/supervisor/desktop.ts): out of the boot bundle of s6-rc, still in
# its database
RUN cd /etc/s6-overlay/s6-rc.d/user/contents.d && rm -f svc-de svc-selkies svc-nginx svc-xorg svc-pulseaudio svc-xsettingsd svc-dbus
# fails the build if the agent cannot load its packages or its page scripts in this image
RUN chmod +x /etc/s6-overlay/s6-rc.d/svc-teamsrelay/run /usr/local/bin/desktop-microphone \
 && node /app/agent.cjs --check \
 && node /app/light.cjs --check \
 && node /app/supervisor.cjs --check

# The web app in Rust (webapp-rs): one static binary. jiff carries the time zone database and reqwest the certificate
# roots, so the image needs no system file. Pinned by digest as calendar/server/agente-rs/Dockerfile.
FROM rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS webrs
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY webapp-rs/Cargo.toml webapp-rs/Cargo.lock ./
COPY webapp-rs/src ./src
# the dependencies and the build stay in the BuildKit cache between deploys; the binary leaves the cache in the same step
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/teamsrelay-web /teamsrelay-web

# Web app: PWA, API, users and sessions, the pages from /ui. Root as the Node image was: data/ and app.db are root's.
# The healthcheck is `/teamsrelay-web --health` (no shell, no curl).
FROM scratch AS web
COPY --from=webrs /teamsrelay-web /teamsrelay-web
COPY --from=build /app/out /ui
ENV PORT=8090 HOSTNAME=0.0.0.0 UI_DIR=/ui
EXPOSE 8090
ENTRYPOINT ["/teamsrelay-web"]
