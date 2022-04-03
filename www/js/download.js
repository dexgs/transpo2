const downloadForm = document.getElementById("download-form");
const downloadButton = document.getElementById("download-button");

async function setupWorker(updateWorker) {
    downloadButton.disabled = true;
    downloadButton.classList.add("throbber");

    try {
        let registration = await navigator.serviceWorker.register("./download_worker.js");

        if (updateWorker) {
            registration = await registration.update();
        }

        await navigator.serviceWorker.ready;

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
    } catch (error) {
        // if the serviceWorker feature is available, but installing failed,
        // try installing again. In the meantime, enable the non-sw download
        // handler so downloading still works.

        console.error(error);
        downloadForm.addEventListener("submit", downloadEventHandler);

        setTimeout(async () => { await setupWorker(true); }, 1000);
    }

    downloadButton.classList.remove("throbber");
    downloadButton.disabled = false;
}


if ("serviceWorker" in navigator) {
    navigator.serviceWorker.addEventListener("message", () => {
        downloadForm.submit();
    });

    downloadForm.addEventListener("submit", async e => {
        e.preventDefault();
        await setupWorker(false);
    });
} else {
    downloadForm.addEventListener("submit", async e => {
        e.preventDefault();

        downloadButton.disabled = true;
        downloadButton.classList.add("throbber");

        const pathElements = location.pathname.split("/");
        await transpoDownload(location.pathname.concat("/dl"));

        downloadButton.classList.remove("throbber");
        downloadButton.disabled = false;
    });
}
