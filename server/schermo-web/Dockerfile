# Schermo del telefono virtuale nel browser (ws-scrcpy, GUIDA.md 4.8): il servizio che tiene il flusso e i tocchi tra il
# browser e il telefono. Non è pubblicato: ci arriva solo il Caddy, dopo l'accesso. L'immagine si costruisce sulla
# macchina che la esegue (arm64 sulla VM).
FROM node:26-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
# nessuno script di installazione: niente compilatore e niente passi nascosti
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY . .
RUN npm run typecheck && npm run dist \
    && cd dist && npm install --omit=dev --ignore-scripts --no-audit --no-fund

FROM node:26-slim
# adb: il client di Android che il servizio usa per parlare con il telefono
RUN apt-get update && apt-get install -y --no-install-recommends adb && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/dist ./
COPY entrypoint.sh /entrypoint.sh
# adb scrive le sue chiavi nella home: /tmp è un tmpfs, il resto del file system è in sola lettura
ENV NODE_ENV=production HOME=/tmp PORT=8000 TELEFONO=telefono:5555
USER 10001
EXPOSE 8000
ENTRYPOINT ["/bin/sh", "/entrypoint.sh"]
