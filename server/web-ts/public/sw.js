// Il service worker delle notifiche del browser (README.md, 4.9): il server manda un testo, la prima riga è il titolo e il resto il corpo.
// Non tiene niente in memoria e non chiama nessun indirizzo. badge è l'icona della barra di stato di Android (bianca su trasparente,
// da app/scripts/genera_icona.py): senza, Chrome ci mette la sua campanella.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("push", (e) => {
  const testo = e.data ? e.data.text() : "";
  const [titolo, ...resto] = testo.split("\n");
  e.waitUntil(self.registration.showNotification(titolo || "Promemoria", { body: resto.join("\n"), icon: "/icon.png", badge: "/badge.png" }));
});

self.addEventListener("notificationclick", (e) => {
  e.notification.close();
  e.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((finestre) => (finestre.length ? finestre[0].focus() : self.clients.openWindow("/"))),
  );
});
