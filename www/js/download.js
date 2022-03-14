const downloadForm = document.getElementById("download-form");
const downloadButton = document.getElementById("download-button");

async function setup() {
    if ("serviceWorker" in navigator) {
        downloadButton.disabled = true;
        downloadButton.classList.add("throbber");

        navigator.serviceWorker.register("./download_worker.js").then(
            async registration => {
                registration = await registration.update();

                downloadButton.classList.remove("throbber");
                downloadButton.disabled = false;

                const key = await transpoGetKeyFromURL();
                const uploadID = transpoGetUploadIDFromURL();

                // send the info the service worker needs to decrypt the request
                // in browser
                const msg = {
                    key: key,
                    uploadID: uploadID
                };

                if (registration.installing) {
                    registration.installing.postMessage(msg);
                } else {
                    registration.active.postMessage(msg);
                }

                console.log("registered service worker");
            },
            error => {
                console.error(error);
                downloadForm.addEventListener("submit", downloadEventHandler);

                downloadButton.classList.remove("throbber");
                downloadButton.disabled = false;
            }
        );
    } else {
        downloadForm.addEventListener("submit", downloadEventHandler);
    }
}

async function downloadEventHandler(e) {
    e.preventDefault();

    downloadButton.disabled = true;
    downloadButton.classList.add("throbber");

    const pathElements = location.pathname.split("/");
    await transpoDownload(location.pathname.concat("/dl"));

    downloadButton.classList.remove("throbber");
    downloadButton.disabled = false;

    return false;
}


window.addEventListener("pageshow", setup);
