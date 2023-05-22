const downloadForm = document.getElementById("download-form");
const downloadButton = document.getElementById("download-button");
const pasteTextOutput = document.getElementById("paste-text-output");
const passwordDialog = document.getElementById("password-dialog");

async function downloadPaste() {
    let password = "";
    const passwordInput = document.getElementById("password-input");
    if (passwordInput) {
        password = encodeURIComponent(passwordInput.value);
    }

    const url = new URL(
        location.origin + location.pathname + "/dl"
        + "?nosw&password=" + password + location.hash);
    const r = await transpoDecryptedResponse(url);

    if (r.ok) {
        pasteTextOutput.value = await r.text();
        return true;
    } else {
        return false;
    }
}

window.onload = async function() {
    if (!hasPassword) {
        await downloadPaste();
    } else {
        downloadForm.addEventListener("submit", async e => {
            e.preventDefault();
            const success = await downloadPaste();
            if (success) {
                passwordDialog.remove();
            }
        });
    }
}
