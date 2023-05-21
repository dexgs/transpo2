const uploadForm = document.getElementById("upload-form");
const sockets = {};
var uploadNum = 0;


function cancelUpload(uploadNum) {
    const socket = sockets[uploadNum];

    if (typeof socket !== typeof undefined) {
        socket.close();
    }

    delete sockets[uploadNum];
}

function closeCallback(close, obj) {
    delete sockets[obj.uploadNum];
}

function errorCallback(error, obj) {
    obj.listItem.classList.add("failed");
    obj.listItem.querySelector(".uploaded-list-item-failed-indicator").hidden = false;
    obj.listItem.querySelector(".uploaded-list-item-copy-url").hidden = true;
    delete sockets[obj.uploadNum];
}

function completionCallback(id, obj) {
    obj.listItem.classList.add("completed");
    obj.listItem.dataset.completed = true;
    obj.progressBar.value = 100;
    delete sockets[obj.uploadNum];
}

function progressCallback(id, bytes, obj) {
    obj.bytesUploaded += bytes;
    const progress = ~~(1000 * obj.bytesUploaded / obj.uploadSize) / 10;

    if (obj.uploadSize > 0 && obj.progressBar.value != progress) {
        obj.progressBar.value = progress;
    }
}

function idCallback(id, key, maxDownloads, password, obj) {
    setUploadedListItemData(obj.listItem, id, key, password !== null);
}

async function upload(e) {
    e.preventDefault();

    // `getFilesToUpload` is different in files.js vs. in paste_upload.js
    const filesToUpload = getFilesToUpload();

    let uploadSize = 0;
    filesToUpload.forEach(file => {
        uploadSize += file.size;
    });

    if (uploadSize * 1.05 > maxUploadSize) {
        return false;
    }

    const formData = new FormData(uploadForm);

    if (filesToUpload.length < 1) {
        return false;
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
    if (formData.get("enable-password") == "on") {
        password = formData.get("password");
    } else {
        password = null;
    }

    let obj = {
        bytesUploaded: 0,
        uploadSize: uploadSize,
        files: filesToUpload,
        socket: null,
        listItem: null,
        progressBar: null,
    };

    let urlPrefix;
    if (location.protocol === 'https:') {
        urlPrefix = "wss://";
    } else {
        urlPrefix = "ws://";
    }

    url = new URL("upload", urlPrefix + location.host + location.pathname).toString();

    obj.socket = await transpoUpload(
        url, filesToUpload, minutes, maxDownloads, password, obj,
        progressCallback, completionCallback, idCallback, errorCallback, closeCallback);

    sockets[uploadNum] = obj.socket;
    obj.uploadNum = uploadNum;
    uploadNum += 1;

    obj.listItem = await addUploadedListItem(filesToUpload, obj.uploadNum);
    obj.progressBar = obj.listItem.querySelector("PROGRESS");

    return false;
}


uploadForm.addEventListener("submit", upload);
