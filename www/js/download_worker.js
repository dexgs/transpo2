self.importScripts("./js/transpo/crypto_for_worker.js");
self.importScripts("./js/transpo/download_for_worker.js");

// needed in case we need to generate a file name
let appName;

self.addEventListener("activate", e => {
    e.waitUntil(self.clients.claim());
});

self.addEventListener("install", e => {
    e.waitUntil(self.skipWaiting());
});

self.addEventListener("message", e => {
    // if the event contains string data
    if (typeof e.data == typeof "") {
        appName = e.data;
        // notify the client that the message was received
        const client = e.source;
        client.postMessage({});
    }
});

self.addEventListener("fetch", e => {
    const url = new URL(e.request.url);
    if (url.pathname.endsWith("/dl") || url.pathname.endsWith("/dl/")) {
        e.respondWith(decryptedResponse(url));
    }
});
