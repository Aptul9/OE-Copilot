# Gateway dell'interfaccia web: pagina di accesso, sessioni e proxy verso il servizio ui.
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
# /dati (gateway.db) è montata da fuori e appartiene all'utente 10001, lo stesso del servizio ui
USER 10001
EXPOSE 3000
CMD ["node", "server.js"]
