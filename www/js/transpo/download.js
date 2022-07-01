import { maxCiphertextSegmentSize, b64Decode, stringToBytes, decrypt, decodeKey } from "./crypto.js";

const textDecoder = new TextDecoder("utf-8");


// Parse the key from the URL fragment
async function getKeyFromURL() {
    const hash = window.location.hash;
    if (hash.length == 0) {
        return null;
    } else {
        return await decodeKey(hash.substring(1));
    }
}

function getUploadIDFromURL() {
    const pathElements = location.pathname.split("/");

    let uploadID = pathElements.pop();
    if (uploadID.length == 0) {
        // if the path ends with "/", the laste element of pathElements will be
        // an empty string, so pop again to get the upload ID.
        uploadID = pathElements.pop();
    }

    return uploadID;
}

function generateFileName(uploadID, mime) {
    let name = appName.concat("_", uploadID);

    if (mime == "application/zip") {
        name = name.concat(".zip");
    }

    return name;
}

async function decryptStream(response, key) { 
    const reader = response.body.getReader();

    let cipherTextLength = 0;
    let totalBytesRead = 0;
    if (response.headers.has("Transpo-Ciphertext-Length")) {
        cipherTextLength = parseInt(response.headers.get("Transpo-Ciphertext-Length"));
    }

    let fileCiphertext = new Uint8Array();

    let segmentBytesRead = 0;
    let segmentBuffer = new Uint8Array(maxCiphertextSegmentSize + 2);
    let segmentWriteStart = 0;
    let segmentSize = 0;

    return new ReadableStream({
        async pull(controller) {
            if (segmentBytesRead >= fileCiphertext.length) {
                const { value, done } = await reader.read();
                if (done) {
                    controller.error(new Error("Download closed prematurely"));
                    return;
                } else {
                    fileCiphertext = value;
                    segmentBytesRead = 0;

                    totalBytesRead += value.length;
                    if (cipherTextLength != 0) {
                        console.debug(totalBytesRead / cipherTextLength);
                    }
                }
            }

            // If size prefix is not in `segmentBuffer`, copy it in.
            if (segmentWriteStart < 2) {
                let iterLen = Math.min(2 - segmentWriteStart, fileCiphertext.length - segmentBytesRead);
                for (let i = 0; i < iterLen; i++) {
                    segmentBuffer[segmentWriteStart + i] = fileCiphertext[segmentBytesRead + i];
                }
                segmentBytesRead += iterLen;
                segmentWriteStart += iterLen;
            }

            if (segmentWriteStart >= 2) {
                segmentSize = segmentBuffer[0] * 256 + segmentBuffer[1];

                if (segmentSize == 0) {
                    controller.close();
                    return;
                } else if (segmentSize > maxCiphertextSegmentSize) {
                    controller.error(new Error("Invalid segment size"));
                    return;
                }
            }

            let iterLen = Math.min(segmentSize + 2 - segmentWriteStart, fileCiphertext.length - segmentBytesRead);
            for (let i = 0; i < iterLen; i++) {
                segmentBuffer[segmentWriteStart + i] = fileCiphertext[segmentBytesRead + i];
            }
            segmentBytesRead += iterLen;
            segmentWriteStart += iterLen;

            // If the whole segment is contained in `segmentBuffer`
            if (segmentWriteStart - 2 == segmentSize) {
                const segmentCiphertext = segmentBuffer.subarray(2, segmentSize + 2);
                const segmentPlaintext = await decrypt(key, segmentCiphertext);

                controller.enqueue(segmentPlaintext);

                segmentSize = 0;
                segmentWriteStart = 0;
            } else {
                controller.enqueue(new Uint8Array(0));
            }
        }
    });
}

// Return a response which wraps an encrypted response and decrypts the contents
// of the contained response.
async function decryptResponse(response, key, uploadID) {
    const mimeCipher = response.headers.get("Content-Type");
    const nameCipher = response.headers.get("Content-Disposition")
        .replace("attachment; filename=", "")
        .replaceAll("\"", "");

    const nameCipherBytes = stringToBytes(b64Decode(nameCipher));
    const mimeCipherBytes = stringToBytes(b64Decode(mimeCipher));

    const nameBytes = await decrypt(key, nameCipherBytes);
    const mimeBytes = await decrypt(key, mimeCipherBytes);

    const mime = textDecoder.decode(mimeBytes);

    let name;
    if (nameBytes.length == 0) {
        // assign a file name if the upload is unnamed
        name = generateFileName(uploadID, mime);
    } else {
        name = textDecoder.decode(nameBytes);
    }

    name = encodeURIComponent(name);

    const stream = await decryptStream(response, key);

    const headers = new Headers();
    headers.append("Content-Type", mime);
    headers.append("Content-Disposition", "attachment; filename=\"" + name + "\"");

    if (response.headers.has("Transpo-Ciphertext-Length")) {
        // A hack, but most browsers make use of Content-Length, even for
        // chunked responses.
        headers.append("Content-Length", response.headers.get("Transpo-Ciphertext-Length"));
    }

    const init = {
        "status": 200,
        "headers": headers
    };

    let decryptedResponse = new Response(stream, init);
    return decryptedResponse;
}

// create a file download prompt for a response
async function downloadResponse(response, uploadID) {
    let name = response.headers.get("Content-Disposition")
        .replace("attachment; filename=", "")
        .replaceAll("\"", "");
    name = decodeURIComponent(name);
    const mime = response.headers.get("Content-Type");

    // assign a file name if the upload is unnamed
    if (name.length == 0) {
        name = generateFileName(uploadID, mime);
    }

    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement("A");

    a.href = url;
    a.download = name;
    a.type = mime;

    document.body.appendChild(a);
    a.click();
    a.remove();
}

async function download(url) {
    const key = await getKeyFromURL();
    const uploadID = getUploadIDFromURL();

    const response = await fetch(url);

    if (response.ok) {
        const decryptedResponse = await decryptResponse(response, key, uploadID);
        await downloadResponse(decryptedResponse, uploadID);

        return true;
    } else {
        return false;
    }
}

if (typeof window != typeof undefined) {
    window.transpoDownload = download;
    window.transpoGetKeyFromURL = getKeyFromURL;
    window.transpoGetUploadIDFromURL = getUploadIDFromURL;
}

export { getKeyFromURL, getUploadIDFromURL, decryptResponse, download, downloadResponse };
