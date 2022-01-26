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
const maxUploadSizeWarning = document.getElementById("max-upload-size-warning");
const maxUploadSizeText = document.getElementById("max-upload-size");


fileInput.addEventListener("input", fileInputEvent);
clearFilesButton.addEventListener("click", clearAllFiles);
document.addEventListener("dragover", e => e.preventDefault());
document.addEventListener("drop", fileDropEvent);


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
    fileInput.files = structuredClone(newList.files);
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
        uploadSizeOutput.innerHTML = "Total Size: " + sizeString(uploadSize);
        fileAreaFooter.style.visibility = "";
    } else {
        fileAreaFooter.style.visibility = "hidden";
    }

    if (uploadSize > maxUploadSize) {
        // Make sure the max upload size text is correct
        maxUploadSizeText.innerHTML = sizeString(maxUploadSize);
        maxUploadSizeWarning.style.display = "";
    } else {
        maxUploadSizeWarning.style.display = "none";
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
    fileInput.files = structuredClone(newList.files);

    updateFileList();
}

function clearAllFiles() {
    let newList = new DataTransfer();

    uploadSize = 0;
    filesToUpload = newList.files;
    fileInput.files = structuredClone(newList.files);

    updateFileList();
}

function fileDropEvent(e) {
    e.preventDefault();
    if (fileArea.contains(e.target)) {
        addFilesToUpload(e.dataTransfer.files);
        updateFileList();
    }
}


window.addEventListener("pageshow", () => {
    // Make sure the file input contains the contents of filesToUpload
    addFilesToUpload([]);
});
