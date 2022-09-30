const downloadForm = document.getElementById("download-form");
const downloadButton = document.getElementById("download-button");

function setButtonDisabled(state) {
    if (state) {
        downloadButton.disabled = true;
        downloadButton.classList.add("throbber");
    } else {
        downloadButton.classList.remove("throbber");
        downloadButton.disabled = false;
    }
}

async function downloadEventHandlerSW(e) {
    e.preventDefault();
    if (!await setupWorkerAndDownload(false)) {
        console.error("Falling back to non-ServiceWorker download");
        await downloadEventHandlerNoSW(e);
    }
}

async function downloadEventHandlerNoSW(e) {
    e.preventDefault();

    const url = new URL(location.origin + location.pathname + "/dl" + location.hash);

    if (!await transpoDownload(url)) {
        window.location = url;
    }
}


async function setupWorker(updateWorker) {
    let registration = await navigator.serviceWorker.register("./download_worker.js");

    if (updateWorker) {
        registration = await registration.update();
    }

    await navigator.serviceWorker.ready;
    navigator.serviceWorker.startMessages();
}

async function setupWorkerAndDownload(updateWorker) {
    try {
        await setupWorker(updateWorker);
        await navigator.serviceWorker.getRegistration();
        navigator.serviceWorker.controller.postMessage(appName);

        // Keep the service worker alive
        pokeWorker();
    } catch (error) {
        console.error(error);
        return false;
    }

    return true;
}

// Firefox will kill the ServiceWorker if it decides that the worker is
// "inactive."
//
// At the time of writing this, firefox decides that the worker is inactive
// even if it contains an active websocket connection. To counter this, we
// send an empty message to the service worker every 5 seconds.
//
// It seems like this resets whatever mechanism Firefox uses to track
// "inactive" workers, which stops it from killing the worker while a download
// is in progress.
function pokeWorker() {
    navigator.serviceWorker.controller.postMessage(new ArrayBuffer(0));
    setTimeout(pokeWorker, 5_000);
}


if ("serviceWorker" in navigator) {
    eventListener = downloadEventHandlerSW;

    // submit a download request when the service worker
    // indicates it's ready by sending back a message.
    navigator.serviceWorker.addEventListener("message", () => {
        downloadForm.submit();
    });

    // add the hash (which stores the key) to the form so that it can
    // be read by the service worker and used to decrypt the download
    downloadForm.action += location.hash;
} else {
    eventListener = downloadEventHandlerNoSW;
}

downloadForm.addEventListener("submit", async e => {
    setButtonDisabled(true);

    try {
        await eventListener(e);
    } catch (error) {
        console.error(error);
    }

    setButtonDisabled(false);
});
