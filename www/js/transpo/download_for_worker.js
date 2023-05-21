// DO NOT USE THIS UNLESS YOU HAVE TO...
//
// This is a version of the `download.js` file in the same directory which is not
// an ES6 module. The reason is that the functions from this file are needed in
// a service worker, but not every browser (looking at you, firefox) supports
// ES6 modules in web workers/service workers.


const textDecoder = new TextDecoder("utf-8");

const ENQUEUE_TARGET = 1_000_000_000_000;


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
    } else if (mime == "text/plain") {
        name = name.concat(".txt");
    }

    return name;
}

// `state` is an object with the following fields:
// - `segment` buffer into which ciphertext is written
// - `segmentWriteStart` index into segment where next read should be inserted
// - `count` number of decryptions so far
// Returns whether or not the full download has been decrypted
async function decryptBufferAndEnqueue(buffer, controller, key, state) {
    const EMPTY = new Uint8Array(0);

    let chunksEnqueued = 0;
    // iterate while the segment buffer can be parsed OR there is still
    // data avialable to read
    let bytesRead = 0;

    while (bytesRead < buffer.byteLength) {
        const remainingSegment = state.segment.byteLength - state.segmentWriteStart;
        const remainingBuffer = buffer.byteLength - bytesRead;
        const iterLen = Math.min(remainingSegment, remainingBuffer);

        for (let i = 0; i < iterLen; i++) {
            state.segment[state.segmentWriteStart + i] = buffer[bytesRead + i];
        }

        state.segmentWriteStart += iterLen;
        bytesRead += iterLen;

        while (state.segmentWriteStart >= 2) {
            const segmentSize = state.segment[0] * 256 + state.segment[1];

            if (segmentSize == 0) {
                if (typeof controller.terminate == typeof undefined) {
                    controller.close();
                } else {
                    controller.terminate();
                }
                return true;
            } else if (state.segmentSize > maxCiphertextSegmentSize) {
                controller.error(new Error("Invalid segment size"));
                return false;
            }

            if (state.segmentWriteStart >= segmentSize + 2) {
                const segmentCiphertext = state.segment.subarray(2, segmentSize + 2);
                const segmentPlaintext = await decrypt(key, state.count, segmentCiphertext);
                state.count++;
                controller.enqueue(segmentPlaintext);
                chunksEnqueued++;

                const segmentEnd = segmentSize + 2;
                const leftover = state.segmentWriteStart - segmentEnd;
                for (let i = 0; i < leftover; i++) {
                    state.segment[i] = state.segment[segmentEnd + i];
                }
                state.segmentWriteStart -= segmentEnd;
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

    return false;
}

async function decryptedStream(url) {
    const key = await getKeyFromURL(url);

    let segment = new Uint8Array(2 + maxCiphertextSegmentSize);
    let segmentWriteStart = 0;
    // count starts at 2 since we first decrypt file name and mime type
    let count = 2;

    let state = {
        'segment': segment,
        'segmentWriteStart': segmentWriteStart,
        'count': count
    };

    const r = await fetch(url);

    let stream;

    if (typeof TransformStream == typeof undefined) {
        // If TransformStream is unavailable, use ReadableStream
        const reader = r.body.getReader();
        stream = new ReadableStream({
            async pull(controller) {
                const { done, value } = await reader.read();
                if (done) {
                    controller.error(new Error("Download failed"));
                } else {
                    if (await decryptBufferAndEnqueue(value, controller, key, state)) {
                        controller.close();
                    }
                }
            }
        });
    } else {
        let finished = false;

        stream = r.body.pipeThrough(new TransformStream({
            async transform(buffer, controller) {
                finished = await decryptBufferAndEnqueue(buffer, controller, key, state);
            },

            async flush(controller) {
                if (!finished) {
                    controller.error(new Error("Download failed"));
                }
            }
        }, {
            highWaterMark: 1
        }, {
            highWaterMark: 1
        }));
    }

    return stream;
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
    if (info.size > 0) {
        headers.append("Content-Length", String(info.size));
    }

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
