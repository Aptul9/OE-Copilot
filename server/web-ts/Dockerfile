# Gateway dell'interfaccia web: pagina di accesso, sessioni, metodi dell'API in TypeScript e loghi.
# L'immagine si costruisce sulla macchina che la esegue (arm64 sulla VM).
FROM node:26-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
# better-sqlite3 porta i binari già compilati: niente script di installazione, niente compilatore
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY . .
ENV NEXT_TELEMETRY_DISABLED=1
RUN npx next build

FROM node:26-slim
WORKDIR /app
ENV NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 PORT=3000 HOSTNAME=0.0.0.0
COPY --from=build /app/.next/standalone ./
COPY --from=build /app/.next/static ./.next/static
COPY --from=build /app/public ./public
# i loghi che porta l'immagine (personale.png, l'icona degli eventi personali): il gateway li cerca dopo quelli di /dati/logos
COPY --from=build /app/logos-predefiniti ./logos-predefiniti
# /dati (gateway.db, loghi delle aziende) è montata da fuori e appartiene all'utente 10001
USER 10001
EXPOSE 3000
CMD ["node", "server.js"]
