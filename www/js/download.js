const downloadForm = document.getElementById("download-form");
const downloadButton = document.getElementById("download-button");


async function downloadEventHandlerSW(e) {
    e.preventDefault();
    if (!await setupWorkerAndDownload(false)) {
        await downloadEventHandlerNoSW(e);
    }
}

async function downloadEventHandlerNoSW(e) {
    e.preventDefault();

    downloadButton.disabled = true;
    downloadButton.classList.add("throbber");

    const pathElements = location.pathname.split("/");
    await transpoDownload(location.pathname.concat("/dl"));

    downloadButton.classList.remove("throbber");
    downloadButton.disabled = false;
}


async function setupWorkerAndDownload(updateWorker) {
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
        console.error(error);
        return false;
    }

    downloadButton.classList.remove("throbber");
    downloadButton.disabled = false;

    return true;
}


const isIOS = /iphone|ipod|ipad/.test(window.navigator.userAgent.toLowerCase());

if ("serviceWorker" in navigator && !isIOS) {
    navigator.serviceWorker.addEventListener("message", () => {
        downloadForm.submit();
    });

    downloadForm.addEventListener("submit", downloadEventHandlerSW);
} else {
    downloadForm.addEventListener("submit", downloadEventHandlerNoSW);
}
