let filesToUpload = new Array();
let uploadSize = 0;
let maxUploadSize = 0;

const fileArea = document.getElementById("file-area");
const fileList = document.getElementById("file-list");
const fileListItemTemplate = document.getElementById("file-list-item-template");
const fileInput = document.getElementById("file-input");
const fileAreaFooter = document.getElementById("file-area-footer");
const clearFilesButton = document.getElementById("clear-files-button");
const uploadSizeOutput = document.getElementById("upload-size-output");
const maxUploadSizeWarningTemplate = document.getElementById("max-upload-size-warning-template");

const uploadedList = document.getElementById("uploaded-list");
const uploadedListItemTemplate = document.getElementById("uploaded-list-item-template");


fileInput.addEventListener("input", fileInputEvent);
clearFilesButton.addEventListener("click", clearAllFiles);
document.addEventListener("dragover", e => e.preventDefault());
document.addEventListener("drop", fileDropEvent);


function copyFileList(files) {
    let newList = new DataTransfer();

    Array.from(files).forEach(file => {
        newList.items.add(file);
    });

    return newList.files;
}

function addFilesToUpload(files) {
    let newFiles = Array.from(files);
    let oldFiles = Array.from(filesToUpload);
    let oldFilesNames = oldFiles.map(file => file.name);
    let newList = new DataTransfer();
    let newUploadSize = 0;

    oldFiles.forEach(file => {
        newUploadSize += file.size;
        newList.items.add(file);
    });
    newFiles.forEach(file => {
        if (!oldFilesNames.includes(file.name)) {
            newUploadSize += file.size;
            newList.items.add(file);
        }
    });

    uploadSize = newUploadSize;
    filesToUpload = newList.files;
    fileInput.files = copyFileList(newList.files);
}

// When the value of the file list is changed, instead of replacing previously
// selected files, append to the end of the file list.
function fileInputEvent(e) {
    addFilesToUpload(e.target.files);

    updateFileList();
}

// Re-create the contents of the file list to reflect the currently selected
// files to upload.
function updateFileList() {
    fileList.innerHTML = "";

    let files = Array.from(filesToUpload);

    files.forEach((file, index) => {
        let listItem = fileListItemTemplate.content.cloneNode(true).firstElementChild;
        listItem.dataset.index = index;
        let itemName = listItem.querySelector("span.file-list-item-name");
        itemName.innerHTML = file.name;
        itemName.title = file.name;
        listItem.querySelector("span.file-list-item-size").innerHTML = sizeString(file.size);
        listItem.querySelector("button.file-list-item-remove")
            .addEventListener("click", () => {fileRemoveEvent(index)});

        fileList.appendChild(listItem);
    });

    if (files.length > 0) {
        uploadSizeOutput.innerHTML = sizeString(uploadSize);
        fileAreaFooter.style.visibility = "";
    } else {
        fileAreaFooter.style.visibility = "hidden";
    }

    if (uploadSize > maxUploadSize) {
        if (!(fileArea.querySelector("div.max-upload-size-warning"))) {
            // Make sure the max upload size text is correct
            let maxUploadSizeWarning = maxUploadSizeWarningTemplate.content.cloneNode(true).firstElementChild;
            let maxUploadSizeText = maxUploadSizeWarning.querySelector("output");
            maxUploadSizeText.innerHTML = sizeString(maxUploadSize);
            fileArea.appendChild(maxUploadSizeWarning);
        }
    } else {
        let maxUploadSizeWarning = fileArea.querySelector("div.max-upload-size-warning");
        if (maxUploadSizeWarning) {
            maxUploadSizeWarning.remove();
        }
    }
}

// Remove the file with the given index from the list of files to upload.
function fileRemoveEvent(index) {
    let oldFiles = Array.from(filesToUpload);
    let newList = new DataTransfer();

    uploadSize -= oldFiles[index].size;

    oldFiles.splice(index, 1);
    oldFiles.forEach(file => newList.items.add(file));

    filesToUpload = newList.files;
    fileInput.files = copyFileList(newList.files);

    updateFileList();
}

function clearAllFiles() {
    let newList = new DataTransfer();

    uploadSize = 0;
    filesToUpload = newList.files;
    fileInput.files = copyFileList(newList.files);

    updateFileList();
}

function fileDropEvent(e) {
    e.preventDefault();
    if (fileArea.contains(e.target)) {
        addFilesToUpload(e.dataTransfer.files);
        updateFileList();
    }
}


// Add an entry to the "uploaded" list
function addUploadedListItem(files, id, key, hasPassword) {
    const listItem = uploadedListItemTemplate.content.cloneNode(true).firstElementChild;
    const link = listItem.querySelector("A");
    const fileName = link.querySelector(".uploaded-list-item-file-name");
    const otherFiles = link.querySelector(".uploaded-list-item-other-files");
    const copyButton = listItem.querySelector(".uploaded-list-item-copy-url");
    const removeButton = listItem.querySelector(".uploaded-list-item-remove");

    listItem.dataset.id = id;

    let allFileNames = files[0].name;
    for (let i = 1; i < files.length; i++) {
        allFileNames = allFileNames.concat(", ", files[i].name);
    }

    link.title = allFileNames;
    if (hasPassword) {
        link.href = id.concat("#", key);
    } else {
        link.href = id.concat("?nopass", "#", key);
    }

    fileName.innerHTML = files[0].name;

    let numOtherFiles = files.length - 1;
    if (numOtherFiles > 0) {
        otherFiles.querySelector("OUTPUT").innerHTML = new String(numOtherFiles);
    } else {
        otherFiles.remove();
    }

    uploadedList.appendChild(listItem);

    return listItem;
}

function copyUploadURL(button) {
    let link = button.parentElement.querySelector("A");

    let textArea = document.createElement("TEXTAREA");
    textArea.value = link.href;

    link.appendChild(textArea);

    textArea.select();
    document.execCommand("copy");
    textArea.remove();
}

function removeUploadedListEntry(button) {
    const id = button.parentElement.dataset.id;

    cancelUpload(id);

    button.parentElement.remove();
}

window.addEventListener("pageshow", () => {
    // Make sure the file input contains the contents of filesToUpload
    addFilesToUpload([]);
});
