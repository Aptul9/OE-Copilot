# Agente di Oe Copilot in TypeScript: legge il calendario del telefono e lo serve al gateway con un'API a chiave (stessa API di server/agent).
# Non è il servizio in uso: parte solo col profilo "agente-ts" di compose (GUIDA.md, sezioni 4.4 e 5). L'immagine si costruisce sulla macchina
# che la esegue (arm64 sulla VM). Node 26 esegue i sorgenti TypeScript così come sono e non ci sono dipendenze: niente compilazione, niente npm.
FROM node:26-slim
ENV NODE_ENV=production TZ=Europe/Rome
WORKDIR /app
COPY package.json ./
COPY src ./src
# calendar.db si copia in /tmp (in compose un tmpfs, il resto del file system è in sola lettura); fonti.json, la cartella del calendario del
# telefono e lo stato dei segni di vita sono montati da fuori, in sola lettura
USER 10001
EXPOSE 8180
CMD ["node", "src/main.ts"]
