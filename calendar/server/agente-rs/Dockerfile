# Agente di Oe Copilot in Rust: la riscrittura di agente-ts con la stessa API, le stesse variabili d'ambiente e lo stesso stato. L'immagine si
# costruisce sulla macchina che la esegue e contiene solo il binario statico: jiff porta con sé il database dei fusi e reqwest le radici dei
# certificati, quindi niente file di sistema, niente shell e niente printenv. Il controllo di salute è `/agente --salute`.
# Fissata per digest (indice amd64 e arm64): per aggiornarla `docker buildx imagetools inspect rust:1-alpine` e si riscrive lo sha256.
FROM rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY nucleo ./nucleo
COPY rete ./rete
COPY agente ./agente
# le dipendenze e la compilazione restano nella cache di BuildKit fra un deploy e l'altro; il binario esce dalla cache nello stesso passo
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --release --locked -p oe-agente && cp target/release/agente /agente

FROM scratch
COPY --from=build /agente /agente
ENV TZ=Europe/Rome
# calendar.db si copia in /tmp (in compose un tmpfs, il resto del file system è in sola lettura); fonti.json, la cartella del calendario del
# telefono e lo stato dei segni di vita sono montati da fuori
USER 10001
EXPOSE 8180
ENTRYPOINT ["/agente"]
