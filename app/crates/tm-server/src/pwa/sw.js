const CACHE_NAME = "tm-mobile-shell-v7";
const SHELL = [
  "/mobile/",
  "/mobile/app.js",
  "/mobile/styles.css",
  "/mobile/manifest.webmanifest",
  "/mobile/icon.svg",
  "/mobile/icon-256.png",
  "/mobile/icon-512.png",
  "/mobile/stock-catalog.json"
];

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(CACHE_NAME).then((cache) => cache.addAll(SHELL)));
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys().then((keys) => Promise.all(
      keys.filter((key) => key !== CACHE_NAME).map((key) => caches.delete(key))
    ))
  );
  self.clients.claim();
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin || url.pathname.startsWith("/api/")) return;
  if (request.method !== "GET") return;
  if (request.mode === "navigate" && url.pathname.startsWith("/mobile")) {
    event.respondWith(fetch(request).catch(() => caches.match("/mobile/")));
    return;
  }
  if (SHELL.includes(url.pathname)) {
    event.respondWith(caches.match(request).then((cached) => cached || fetch(request)));
  }
});
