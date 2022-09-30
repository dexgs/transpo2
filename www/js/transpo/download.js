import { maxCiphertextSegmentSize, b64Decode, stringToBytes, decrypt, decodeKey } from "./crypto.js";

const textDecoder = new TextDecoder("utf-8");


// Parse the key from the URL fragment
async function getKeyFromURL(url) {
    const hash = url.hash;
    if (hash.length == 0) {
        return null;
    } else {
        return await decodeKey(hash.substring(1));
    }
}

function getUploadIDFromURL(url) {
    const pathElements = url.pathname.split("/");

    let uploadID = pathElements.pop();
    // strip off non-id parts of the path
    while (uploadID == "" || uploadID == "dl" || uploadID == "dlws") {
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

async function decryptedStream(url) {
    const key = await decodeKey(url.hash.substring(1));

    const socket = new WebSocket(
        url.origin.replace("http", "ws")
        + url.pathname.replace(/\/$/, "")
        + "ws" + url.search);
    socket.binaryType = "arraybuffer";

    await new Promise(resolve => {
        socket.onopen = resolve;
    });

    let receivedBuffer = null;
    let promise = null;
    let wsResolve = null;

    socket.addEventListener("message", msg => {
        receivedBuffer = new Uint8Array(msg.data);
        wsResolve();
    });

    socket.addEventListener("close", msg => {
        wsResolve();
    });

    const EMPTY = new Uint8Array(0);

    let segment = new Uint8Array(2 + maxCiphertextSegmentSize);
    let segmentWriteStart = 0;
    // count starts at 2 since we first decrypt file name and mime type
    let count = 2;

    promise = new Promise(resolve => {
        wsResolve = resolve;
    });
    socket.send(EMPTY);

    return new ReadableStream({
        async pull(controller) {
            await promise;

            let buffer = receivedBuffer;
            receivedBuffer = null;

            if (buffer == null) {
                buffer = new Uint8Array(2);
            }

            promise = new Promise(resolve => {
                wsResolve = resolve;
            });
            socket.send(EMPTY);

            let chunksEnqueued = 0;

            // iterate while the segment buffer can be parsed OR there is still
            // data avialable to read
            let bytesRead = 0;
            while (bytesRead < buffer.byteLength) {
                const remainingSegment = segment.byteLength - segmentWriteStart;
                const remainingBuffer = buffer.byteLength - bytesRead;
                const iterLen = Math.min(remainingSegment, remainingBuffer);

                for (let i = 0; i < iterLen; i++) {
                    segment[segmentWriteStart + i] = buffer[bytesRead + i];
                }

                segmentWriteStart += iterLen;
                bytesRead += iterLen;

                while (segmentWriteStart >= 2) {
                    const segmentSize = segment[0] * 256 + segment[1];

                    if (segmentSize == 0) {
                        controller.close();
                        return;
                    } else if (segmentSize > maxCiphertextSegmentSize) {
                        controller.error(new Error("Invalid segment size"));
                        return;
                    }

                    if (segmentWriteStart >= segmentSize + 2) {
                        const segmentCiphertext = segment.subarray(2, segmentSize + 2);
                        const segmentPlaintext = await decrypt(key, count, segmentCiphertext);
                        count++;
                        controller.enqueue(segmentPlaintext);
                        chunksEnqueued++;

                        const segmentEnd = segmentSize + 2;
                        const leftover = segmentWriteStart - segmentEnd;
                        for (let i = 0; i < leftover; i++) {
                            segment[i] = segment[segmentEnd + i];
                        }
                        segmentWriteStart -= segmentEnd;
                    } else {
                        break;
                    }
                }
            }

            // Every call to pull is expected to enqueue something, so if we
            // didn't decrypt any chunks this call, just enqueue the empty
            // array as this will satisfy the caller without actually messing
            // up the downloaded file.
            if (chunksEnqueued == 0) {
                controller.enqueue(EMPTY);
            }
        }
    });
}

async function decryptedResponse(url) {
    const key = await getKeyFromURL(url);
    const uploadID = getUploadIDFromURL(url);

    const r = await fetch(uploadID + "/info" + url.search);
    if (!r.ok) {
        return r;
    }

    const info = await r.json();
    const nameCipherBytes = stringToBytes(b64Decode(info.name));
    const mimeCipherBytes = stringToBytes(b64Decode(info.mime));

    const nameBytes = await decrypt(key, 0, nameCipherBytes);
    const mimeBytes = await decrypt(key, 1, mimeCipherBytes);

    const mime = textDecoder.decode(mimeBytes);

    let name;
    if (nameBytes.length == 0) {
        // assign a file name if the upload is unnamed
        name = generateFileName(uploadID, mime);
    } else {
        name = textDecoder.decode(nameBytes);
    }
    name = encodeURIComponent(name);

    const headers = new Headers();
    headers.append("Content-Type", mime);
    headers.append("Content-Disposition", "attachment; filename=\"" + name + "\"");
    headers.append("Content-Length", String(info.size));

    const init = {
        "status": 200,
        "headers": headers
    };

    const stream = await decryptedStream(url);

    return new Response(stream, init);
}

// create a file download prompt for a response
async function downloadResponse(response, url) {
    const uploadID = getUploadIDFromURL(url);

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
    const blobHref = URL.createObjectURL(blob);
    const a = document.createElement("A");

    a.href = blobHref;
    a.download = name;
    a.type = mime;

    document.body.appendChild(a);
    a.click();
    a.remove();
}

async function download(url) {
    const response = await decryptedResponse(url);

    if (response.ok) {
        await downloadResponse(response, url);
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

export { getKeyFromURL, getUploadIDFromURL, decryptedResponse, download, downloadResponse };
