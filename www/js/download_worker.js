self.importScripts("./js/transpo/crypto_for_worker.js");
self.importScripts("./js/transpo/download_for_worker.js");


let key;
let uploadID;
let appName;

self.addEventListener("activate", e => {
  e.waitUntil(clients.claim());
});

self.addEventListener("install", e => {
    self.skipWaiting();
});

self.addEventListener("message", e => {
    key = e.data.key;
    uploadID = e.data.uploadID;
    appName = e.data.appName;
});

self.addEventListener("fetch", e => {
    const url = new URL(e.request.url);
    if (url.pathname.endsWith("/dl") || url.pathname.endsWith("/dl/")) {
        e.respondWith(fetch(url.href)
            .then(r => {
                if (r.ok) {
                    return decryptResponse(r, key, uploadID);
                } else {
                    return r;
                }
            })
        );
    }
});
