# Agente di Oe Copilot: legge i calendari delle caselle e li serve al gateway con un'API a chiave. È il servizio agent di compose (README.md,
# sezioni 4.4 e 5). L'immagine si costruisce sulla macchina che la esegue (arm64 sulla VM). Node 26 esegue i sorgenti TypeScript così come
# sono, senza compilazione. protobufjs legge lo schema binario di Google Chat.
# Fissata per digest (indice amd64 e arm64): per aggiornarla `docker buildx imagetools inspect node:26-slim` e si riscrive lo sha256 in tutti i FROM node di agente-ts e web-ts. Su tag mobile resta caddy-ui (`caddy:2.11.4-alpine`).
FROM node:26-slim@sha256:193fe51b64e77981119c98c2002c9e32a70e2f006fb4d25068ce0558998917f0
ENV NODE_ENV=production TZ=Europe/Rome
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev --ignore-scripts --no-audit --no-fund
COPY proto ./proto
COPY src ./src
# calendar.db si copia in /tmp (in compose un tmpfs, il resto del file system è in sola lettura); fonti.json, la cartella del calendario del
# telefono e lo stato dei segni di vita sono montati da fuori, in sola lettura
USER 10001
EXPOSE 8180
CMD ["node", "src/main.ts"]
