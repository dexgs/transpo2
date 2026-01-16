// DO NOT USE THIS UNLESS YOU HAVE TO...
//
// This is a version of the `download.js` file in the same directory which is not
// an ES6 module. The reason is that the functions from this file are needed in
// a service worker, but not every browser (looking at you, firefox) supports
// ES6 modules in web workers/service workers.


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
    } else if (mime == "text/plain") {
        name = name.concat(".txt");
    }

    return name;
}

function mergeBuffers(b1, b2) {
    const merged = new Uint8Array(b1.length + b2.length);
    merged.set(b1);
    merged.set(b2, b1.length);
    return merged;
}

async function sizedRead(reader, buffer, size) {
    while (buffer.length < size) {
        const { done, value } = await reader.read();
        if (done) {
            throw new Error("Unexpected end of stream");
        }
        buffer = mergeBuffers(buffer, value);
    }

    return buffer;
}

// Read a single length-prefixed chunk
async function readChunk(reader, buffer, maxLength) {
    buffer = await sizedRead(reader, buffer, 2);
    const length = buffer[0] * 256 + buffer[1];
    if (length > maxLength) {
        throw new Error("Chunk too long");
    }
    buffer = buffer.subarray(2);

    buffer = await sizedRead(reader, buffer, length);
    return {
        'chunk': buffer.subarray(0, length),
        'leftover': buffer.subarray(length)
    };
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
    const reader = r.body.getReader();

    // First, read the file name and mime type
    let nameChunk;
    let mimeChunk;
    let leftover = new Uint8Array(0);
    ({ 'chunk': nameChunk, leftover } = await readChunk(reader, leftover, 512));
    ({ 'chunk': mimeChunk, leftover } = await readChunk(reader, leftover, 272));
    const nameBytes = await decrypt(key, 0, nameChunk);
    const mimeBytes = await decrypt(key, 1, mimeChunk);
    const name = textDecoder.decode(nameBytes);
    const mime = textDecoder.decode(mimeBytes);

    // Then, read the file contents
    const state = {
        'segment': new Uint8Array(2 + maxCiphertextSegmentSize),
        'segmentWriteStart': 0,
        'count': 2, // count starts at 2 since we first decrypt file name and mime type
        'isFinished': false
    };

    let stream;
    if (typeof TransformStream == typeof undefined) {
        // If TransformStream is unavailable, use ReadableStream
        stream = new ReadableStream({
            async start(controller) {
                // handle any leftover data from reading the name and mime type
                await decryptBufferAndEnqueue(leftover, controller, key, state);
            },

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
        // We *must* give up the reader before piping through TransformStream
        reader.releaseLock();
        stream = r.body.pipeThrough(new TransformStream({
            async start(controller) {
                // handle any leftover data from reading the name and mime type
                await decryptBufferAndEnqueue(leftover, controller, key, state);
            },

            async transform(buffer, controller) {
                await decryptBufferAndEnqueue(buffer, controller, key, state);
            }
        }));
    }

    return {
        'stream': stream,
        'name': name,
        'mime': mime
    };
}


async function decryptedResponse(url) {
    const key = await getKeyFromURL(url);
    const uploadID = getUploadIDFromURL(url);

    const r = await fetch(url);
    if (!r.ok) {
        return r;
    }

    let { stream, name, mime } = await decryptedStream(r, key);
    if (name.length == 0) {
        // assign a file name if the upload is unnamed
        name = generateFileName(uploadID, mime);
    }
    name = encodeURIComponent(name);

    const length = r.headers.get("Transpo-Ciphertext-Length");

    const headers = new Headers();
    headers.append("Content-Type", mime);
    headers.append("Content-Disposition", "attachment; filename=\"" + name + "\"");
    if (length > 0) {
        headers.append("Content-Length", length);
    }

    const init = {
        "status": 200,
        "headers": headers
    };
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
