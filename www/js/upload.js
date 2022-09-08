const uploadForm = document.getElementById("upload-form")
const sockets = {};
var uploadNum = 0;


function cancelUpload(uploadNum) {
    const socket = sockets[uploadNum];

    if (typeof socket !== typeof undefined) {
        socket.send("CANCEL");
        socket.close();
    }

    delete sockets[uploadNum];
}

function closeCallback(close, obj) {
    // If the socket was closed prematurely, treat it as an error
    if (!obj.isCompleted) {
        errorCallback(null, obj);
    }

    delete sockets[obj.uploadNum];
}

function errorCallback(error, obj) {
    if (!obj.isCompleted) {
        obj.listItem.classList.add("failed");
    }

    delete sockets[obj.uploadNum];
}

function completionCallback(id, obj) {
    obj.listItem.classList.add("completed");
    obj.isCompleted = true;
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

    if (uploadSize > maxUploadSize) {
        return false;
    }

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
    if (formData.get("enable-password") == "on") {
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

    sockets[uploadNum] = obj.socket;
    obj.uploadNum = uploadNum;
    uploadNum += 1;

    obj.listItem = addUploadedListItem(files, obj.uploadNum);
    obj.progressBar = obj.listItem.querySelector("PROGRESS");

    return false;
}


uploadForm.addEventListener("submit", upload);
