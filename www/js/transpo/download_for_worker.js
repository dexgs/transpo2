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
// - `isFinished` whether or not the download is finished
// Returns the number of chunks enqueued
async function decryptBufferAndEnqueue(buffer, controller, key, state) {
    // Fully consume `buffer` (the ciphertext) and decrypt as much as
    // possible. Then, copy any leftovers to the start of `state.segment`
    // and update `state.segmentWriteStart` so the next buffer gets inserted
    // at the right place.

    let chunksEnqueued = 0;

    let bufferReadStart = 0;
    while (bufferReadStart < buffer.byteLength) {
        const remainingSegment = state.segment.byteLength - state.segmentWriteStart;
        const remainingBuffer = buffer.byteLength - bufferReadStart;
        const copyLen = Math.min(remainingSegment, remainingBuffer);
        if (copyLen == 0) {
            controller.error(new Error("copied 0 bytes from buffer"));
            return;
        }

        // Copy from `buffer` into `segment`
        const copyFrom = buffer.slice(bufferReadStart, bufferReadStart + copyLen);
        state.segment.set(copyFrom, state.segmentWriteStart);
        state.segmentWriteStart += copyLen;
        bufferReadStart += copyLen;

        let segmentReadStart = 0;
        const segmentReadEnd = state.segmentWriteStart;

        // Decrypt and enqueue as many plaintext chunks as we can
        while (segmentReadStart + 2 <= segmentReadEnd) {
            const segmentSize =
                state.segment[segmentReadStart] * 256
                + state.segment[segmentReadStart + 1];

            if (segmentSize == 0) {
                // End of file indicated by zero-length segment
                if (typeof controller.terminate == typeof undefined) {
                    controller.close();
                } else {
                    controller.terminate();
                }
                state.isFinished = true;
                break;
            } else if (segmentSize > maxCiphertextSegmentSize) {
                controller.error(new Error("Segment too big"));
                break;
            } else if (segmentReadStart + 2 + segmentSize > segmentReadEnd) {
                // Segment is incomplete
                break;
            } else {
                // Get the next ciphertext segment
                const segmentStart = segmentReadStart + 2;
                const segmentEnd = segmentStart + segmentSize;
                const segmentCiphertext = state.segment.subarray(segmentStart, segmentEnd);

                // Decrypt and enqueue
                const segmentPlaintext = await decrypt(key, state.count, segmentCiphertext);
                state.count++;
                controller.enqueue(segmentPlaintext);
                chunksEnqueued++;

                // Advance
                segmentReadStart = segmentEnd;
            }
        }

        if (segmentReadStart > 0) {
            // Move any "leftover" data to the start of the segment to make room
            // for the next copy from `buffer` into segment
            state.segment.copyWithin(0, segmentReadStart, state.segmentWriteStart);
            state.segmentWriteStart -= segmentReadStart;
        }
    }

    return chunksEnqueued;
}

async function decryptedStream(r, key) {
    let segment = new Uint8Array(2 + maxCiphertextSegmentSize);
    let segmentWriteStart = 0;
    // count starts at 2 since we first decrypt file name and mime type
    let count = 2;

    let state = {
        'segment': segment,
        'segmentWriteStart': segmentWriteStart,
        'count': count,
        'isFinished': false
    };

    let stream;

    if (typeof TransformStream == typeof undefined) {
        // If TransformStream is unavailable, use ReadableStream
        const reader = r.body.getReader();
        stream = new ReadableStream({
            async pull(controller) {
                // `pull` is expected to enqueue _something_, so loop until
                // the download finishes, or we enqueue a non-zero number
                // of chunks
                while (true) {
                    const { done, value } = await reader.read();
                    if (done) {
                        controller.error(new Error("Download failed"));
                        break;
                    } else {
                        let n = await decryptBufferAndEnqueue(value, controller, key, state);

                        if (state.isFinished) {
                            controller.close();
                            break;
                        } else if (n > 0) {
                            break;
                        }
                    }
                }
            }
        });
    } else {
        stream = r.body.pipeThrough(new TransformStream({
            async transform(buffer, controller) {
                await decryptBufferAndEnqueue(buffer, controller, key, state);
            }
        }));
    }

    return stream;
}


async function decryptedResponse(url) {
    const key = await getKeyFromURL(url);
    const uploadID = getUploadIDFromURL(url);

    let r = await fetch(uploadID + "/info" + url.search);
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

    r = await fetch(url);
    if (r.ok) {
        const stream = await decryptedStream(r, key);

        const init = {
            "status": 200,
            "headers": headers
        };
        return new Response(stream, init);
    } else {
        return r;
    }
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
