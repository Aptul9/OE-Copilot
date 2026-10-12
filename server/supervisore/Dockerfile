# Supervisore del telefono virtuale (GUIDA.md 4.5 e 6): il giro di sincronizzazione, il battito e il rilevatore degli accessi, con adb sulla
# rete di compose (telefono:5555) al posto di `docker exec`. Non è pubblicato su nessuna porta. L'immagine si costruisce sulla macchina che
# la esegue (arm64 sulla VM).
FROM node:26-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
# nessuno script di installazione: niente compilatore e niente passi nascosti
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY . .
RUN npm run build && npm prune --omit=dev --ignore-scripts --no-audit --no-fund

FROM node:26-slim
# adb: il server di Android che adbkit usa per parlare con il telefono
RUN apt-get update && apt-get install -y --no-install-recommends adb && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/dist ./dist
COPY package.json entrypoint.sh ./
# adb scrive le sue chiavi nella home: /tmp è un tmpfs, il resto del file system è in sola lettura. 10080 è l'utente di Outlook sul telefono
# (u0_a80): solo lui e root leggono i suoi dati, che stanno nelle cartelle del server montate qui
ENV NODE_ENV=production HOME=/tmp OE_DIR=/opt/oe-copilot TELEFONO=telefono:5555
USER 10080
ENTRYPOINT ["/bin/sh", "/app/entrypoint.sh"]
