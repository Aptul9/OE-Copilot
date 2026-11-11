# Gateway di Oe Copilot in Rust: la riscrittura di web-ts con le stesse strade, le stesse variabili d'ambiente e lo stesso gateway.db (sessioni,
# TOTP e passkey restano validi). L'interfaccia React è l'esportazione statica di web-ts (OE_ESPORTA=1, pagine *.esporta.tsx), che il gateway
# serve con i valori della pagina nella testata. Il contesto `web` è la cartella web-ts (docker-compose.yml, additional_contexts).
# L'immagine finale ha il binario statico, l'interfaccia e i loghi predefiniti: niente shell, niente node. Il controllo di salute è `/gateway --salute`.
# Fissate per digest: per aggiornarle `docker buildx imagetools inspect node:26-slim` (o rust:1-alpine) e si riscrive lo sha256.
FROM node:26-slim@sha256:193fe51b64e77981119c98c2002c9e32a70e2f006fb4d25068ce0558998917f0 AS ui
WORKDIR /web
COPY --from=web package.json package-lock.json ./
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY --from=web . .
ENV NEXT_TELEMETRY_DISABLED=1 OE_ESPORTA=1
RUN npx next build && test -f out/index.html && test -f out/login.html && test -f out/404.html

FROM rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY nucleo ./nucleo
COPY rete ./rete
COPY http ./http
COPY agente ./agente
COPY gateway ./gateway
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --release --locked -p oe-gateway && cp target/release/gateway /gateway

FROM scratch
COPY --from=build /gateway /gateway
COPY --from=ui /web/out /ui
COPY --from=web logos-predefiniti /logos-predefiniti
ENV TZ=Europe/Rome UI_STATICA=/ui LOGHI_PREDEFINITI=/logos-predefiniti PORT=3000
# /dati (gateway.db, loghi delle aziende) è montata da fuori e appartiene all'utente 10001
USER 10001
EXPOSE 3000
ENTRYPOINT ["/gateway"]
