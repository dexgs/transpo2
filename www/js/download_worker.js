self.importScripts("./js/transpo/crypto_for_worker.js");
self.importScripts("./js/transpo/download_for_worker.js");

// needed in case we need to generate a file name
let appName;

self.addEventListener("activate", e => {
    self.clients.claim();
});

self.addEventListener("install", e => {
    self.skipWaiting();
});

self.addEventListener("message", async e => {
    appName = e.data;

    // notify the client that the message was received
    const client = e.source;
    client.postMessage({});
});

function getUploadIDFromPath(path) {
    // path is in the format: ".../<upload ID>/dl/"
    if (path.endsWith("/")) {
        path = path.substring(path.length - 1);
    }
    segments = path.split("/");
    return segments[segments.length - 2];
}

function getHashFromURL(url) {
    return url.substring(url.indexOf("#") + 1, url.length);
}

self.addEventListener("fetch", e => {
    const url = new URL(e.request.url);
    if (url.pathname.endsWith("/dl") || url.pathname.endsWith("/dl/")) {
        e.respondWith(fetch(url.origin + url.pathname)
            .then(async r => {
                if (r.ok) {
                    const key = await decodeKey(getHashFromURL(url.href));
                    const uploadID = getUploadIDFromPath(url.pathname);
                    return decryptResponse(r, key, uploadID);
                } else {
                    return r;
                }
            })
        );
    }
});
