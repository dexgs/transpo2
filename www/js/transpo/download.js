import { maxSegmentSize, b64Decode, stringToBytes, decrypt, decodeKey } from "./crypto.js";

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

async function decryptStream(response, key) {
    const reader = response.body.getReader();

    let fileCiphertext = new Uint8Array();
    let bytesRead = 0;

    let segmentBuffer = new Uint8Array(maxSegmentSize + 2);
    let segmentWriteStart = 0;
    let segmentSize = 0;

    return new ReadableStream({
        async pull(controller) {
            if (bytesRead >= fileCiphertext.length) {
                const { value, done } = await reader.read();
                if (done) {
                    controller.close();
                    return;
                } else {
                    fileCiphertext = value;
                    bytesRead = 0;
                }
            }

            // If size prefix is not in `segmentBuffer`, copy it in.
            if (segmentWriteStart < 2) {
                let iterLen = Math.min(2 - segmentWriteStart, fileCiphertext.length - bytesRead);
                for (let i = 0; i < iterLen; i++) {
                    segmentBuffer[segmentWriteStart + i] = fileCiphertext[bytesRead + i];
                }
                bytesRead += iterLen;
                segmentWriteStart += iterLen;
            }

            if (segmentWriteStart >= 2) {
                segmentSize = segmentBuffer[0] * 256 + segmentBuffer[1];

                if (segmentSize > maxSegmentSize || segmentSize == 0) {
                    controller.close();
                    return;
                }
            }

            let iterLen = Math.min(segmentSize, fileCiphertext.length - bytesRead);
            for (let i = 0; i < iterLen; i++) {
                segmentBuffer[segmentWriteStart + i] = fileCiphertext[bytesRead + i];
            }
            bytesRead += iterLen;
            segmentWriteStart += iterLen;

            // If the whole segment is contained in `segmentBuffer`
            if (segmentWriteStart - 2 == segmentSize) {
                const segmentCiphertext = segmentBuffer.subarray(2, segmentSize + 2);
                const segmentPlaintext = await decrypt(key, segmentCiphertext);

                controller.enqueue(segmentPlaintext);

                segmentSize = 0;
                segmentWriteStart = 0;
            }
        }
    });
}

// Return a response which wraps an encrypted response and decrypts the contents
// of the contained response.
async function decryptResponseWithKey(response, key, nameCipher, mimeCipher) {
    const nameCipherBytes = stringToBytes(b64Decode(nameCipher));
    const mimeCipherBytes = stringToBytes(b64Decode(mimeCipher));

    const nameBytes = await decrypt(key, nameCipherBytes);
    const mimeBytes = await decrypt(key, mimeCipherBytes);

    const name = encodeURIComponent(textDecoder.decode(nameBytes));
    const mime = textDecoder.decode(mimeBytes);

    const stream = await decryptStream(response, key);

    const headers = new Headers();
    headers.append("Content-Type", mime);
    headers.append("Content-Disposition", "attachment; filename=\"" + name + "\"");

    const init = {
        "status": 200,
        "headers": headers
    };

    let decryptedResponse = new Response(stream, init);
    return decryptedResponse;
}

async function decryptResponse(response) {
    const mimeCipher = response.headers.get("Content-Type");
    const nameCipher = response.headers.get("Content-Disposition")
        .replace("attachment; filename=", "")
        .replaceAll("\"", "");
    const key = await getKeyFromURL();

    return await decryptResponseWithKey(response, key, nameCipher, mimeCipher);
}

// create a file download prompt for a response
async function downloadResponse(response) {
    let name = response.headers.get("Content-Disposition")
        .replace("attachment; filename=", "")
        .replace("\"", "");
    name = decodeURIComponent(name);
    const mime = response.headers.get("Content-Type");

    // read the upload ID from the path of the current location
    const pathElements = location.pathname.split("/");
    let uploadID = pathElements.pop();
    while (uploadID.length == 0) {
        uploadID = pathElements.pop();
    }

    // assign a file name if the upload is unnamed
    if (name.length == 0) {
        name = appName.concat("_", uploadID);

        if (mime == "application/zip") {
            name = name.concat(".zip");
        }
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
    const response = await fetch(url);
    const decryptedResponse = await decryptResponse(response);
    await downloadResponse(decryptedResponse);
}

window.transpoDownload = download;

export { getKeyFromURL, decryptResponseWithKey, decryptResponse, download };
