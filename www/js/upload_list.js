const uploadedList = document.getElementById("uploaded-list");
const uploadedListItemTemplate = document.getElementById("uploaded-list-item-template");

// Add an entry to the "uploaded" list
async function addUploadedListItem(files, uploadNum) {
    const listItem = uploadedListItemTemplate.content.cloneNode(true).firstElementChild;
    const link = listItem.querySelector("A");
    const fileName = link.querySelector(".uploaded-list-item-file-name");
    const otherFiles = link.querySelector(".uploaded-list-item-other-files");

    listItem.dataset.uploadNum = uploadNum;
    listItem.classList.add("missing-data");

    // This function is defined differently in files.js vs. paste_upload.js
    text = await getListItemText(files);

    link.title = text.title;
    fileName.innerHTML = safeString(text.name);

    let numOtherFiles = files.length - 1;
    if (numOtherFiles > 0) {
        otherFiles.querySelector("OUTPUT").innerHTML = new String(numOtherFiles);
    } else {
        otherFiles.remove();
    }

    uploadedList.appendChild(listItem);

    return listItem;
}

function setUploadedListItemData(listItem, id, key, hasPassword, isPaste) {
    listItem.classList.remove("missing-data");

    const link = listItem.querySelector("A");

    let query = [];
    if (!hasPassword) {
        query.push("nopass");
    }
    if (isPaste) {
        query.push("paste");
    }

    link.href = id;

    if (query.length > 0) {
        link.href += "?";
        link.href += query.join("&");
    }

    link.href += "#";
    link.href += key;
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

function findListEntry(el) {
    // find the list item containing el
    let pElement = el.parentElement;
    while (pElement.nodeName != "LI") {
        pElement = pElement.parentElement;
    }
    return pElement;
}

function removeUploadedListEntry(button) {
    // find the list item containing el
    const listEntry = findListEntry(button);

    if (listEntry.dataset.completed != true) {
        const uploadNum = listEntry.dataset.uploadNum;
        cancelUpload(uploadNum);
    }

    listEntry.remove();
}

function showErrorReason(button) {
    const listEntry = findListEntry(button);
    showError(listEntry.dataset.errorCode);
}
