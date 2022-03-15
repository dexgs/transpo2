const downloadForm = document.getElementById("download-form");
const downloadButton = document.getElementById("download-button");

async function setup(updateWorker) {
    downloadForm.removeEventListener("submit", downloadEventHandler);

    if ("serviceWorker" in navigator) {
        downloadButton.disabled = true;
        downloadButton.classList.add("throbber");

        try {
            let registration = await navigator.serviceWorker.register("./download_worker.js");
            downloadForm.removeEventListener("submit", downloadEventHandler);

            if (updateWorker) {
                registration = await registration.update();
            }

            const key = await transpoGetKeyFromURL();
            const uploadID = transpoGetUploadIDFromURL();

            // send the info the service worker needs to decrypt the request
            // in browser
            const msg = {
                key: key,
                uploadID: uploadID,
                appName: appName
            };

            if (registration.installing) {
                registration.installing.postMessage(msg);
            } else {
                registration.active.postMessage(msg);
            }

            console.log("registered service worker");
        } catch (error) {
            // if the serviceWorker feature is available, but installing failed,
            // try installing again. In the meantime, enable the non-sw download
            // handler so downloading still works.

            console.error(error);
            downloadForm.addEventListener("submit", downloadEventHandler);

            setTimeout(async () => { await setup(true); }, 100);
        }

        downloadButton.classList.remove("throbber");
        downloadButton.disabled = false;
    } else {
        downloadForm.addEventListener("submit", downloadEventHandler);
    }
}

async function downloadEventHandler(e) {
    downloadButton.disabled = true;
    downloadButton.classList.add("throbber");

    const pathElements = location.pathname.split("/");
    const success = await transpoDownload(location.pathname.concat("/dl"));

    downloadButton.classList.remove("throbber");
    downloadButton.disabled = false;

    if (success) {
        e.preventDefault();
        return false;
    }
}


window.addEventListener("pageshow", async () => { await setup(true) });
downloadForm.addEventListener("submit", async () => { await setup(false) });
