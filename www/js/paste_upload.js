const textArea = document.getElementById("paste-text-input");
const textEncoder = new TextEncoder();

const uploadButton = document.getElementById("upload-button");


function getFilesToUpload() {
    if (textArea.value.length > 0) {
        const file = new File([textArea.value], "", { type: "text/plain" });
        return [file];
    } else {
        return [];
    }
}

async function getListItemText(files) {
    let name = (await files[0].text()).replace("\\w+", " ").substring(0, 500);

    return {
        name: name,
        title: ""
    };
}

textArea.addEventListener("input", () => {
    uploadButton.disabled = textArea.textLength == 0;
});
