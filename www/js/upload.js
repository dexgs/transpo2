const uploadForm = document.getElementById("upload-form")
const sockets = {};


function cancelUpload(id) {
    const socket = sockets[id];

    if (typeof socket !== typeof undefined) {
        socket.send("CANCEL");
        socket.close();
    }
}

function closeCallback(id, obj) {
    // If the socket was closed prematurely, treat it as an error
    if (!obj.isCompleted) {
        errorCallback(id, null, obj);
    }
}

function errorCallback(id, error, obj) {
    obj.listItem.classList.add("failed");
    delete sockets[id];
}

function completionCallback(id, obj) {
    obj.listItem.classList.add("completed");
    obj.isCompleted = true;
    delete sockets[id];
}

function progressCallback(id, bytes, obj) {
    obj.bytesUploaded += bytes;

    if (obj.uploadSize > 0) {
        obj.progressBar.value = ~~(100 * obj.bytesUploaded / obj.uploadSize);
    }
}

function idCallback(id, key, maxDownloads, password, obj) {
    obj.listItem = addUploadedListItem(obj.files, id, key, password !== null);
    obj.progressBar = obj.listItem.querySelector("PROGRESS");
    sockets[id] = obj.socket;
}

async function upload(e) {
    e.preventDefault();

    const files = filesToUpload;
    const formData = new FormData(uploadForm);

    if (filesToUpload.length < 1) {
        return;
    }

    const minutes = 
        (~~formData.get("days")) * (24 * 60)
        + (~~formData.get("hours")) * 60
        + (~~formData.get("minutes"));

    let maxDownloads;
    if (formData.get("enable-max-downloads")) {
        maxDownloads = ~~formData.get("max-downloads");
    } else {
        maxDownloads = null;
    }

    let password;
    if (formData.get("enable-password")) {
        password = formData.get("password");
    } else {
        password = null;
    }

    let obj = {
        bytesUploaded: 0,
        uploadSize: uploadSize,
        files: files,
        socket: null,
        listItem: null,
        progressBar: null,
        isCompleted: false
    };

    let urlPrefix;
    if (location.protocol === 'https:') {
        urlPrefix = "wss://";
    } else {
        urlPrefix = "ws://";
    }

    url = urlPrefix + location.host + location.pathname;

    if (location.pathname.endsWith("/")) {
        url = url + "upload";
    } else {
        url = url + "/upload";
    }

    obj.socket = await transpoUpload(
        url, files, minutes, maxDownloads, password, obj,
        progressCallback, completionCallback, idCallback, errorCallback, closeCallback);

    return false;
}


uploadForm.addEventListener("submit", upload);
