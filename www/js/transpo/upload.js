import { maxPlaintextSegmentSize, b64Encode, encrypt, genKey, encodeKey } from "./crypto.js";
import { downloadZip } from "./client-zip/index.js";

const textEncoder = new TextEncoder("utf-8");

const MAX_BUFFERED_AMOUNT = 10_000_000;
const MAX_SEND_WAIT_MS = 100;


async function encryptStream(files, key) {
    let fileStream;
    if (files.length == 1) {
        fileStream = files[0].stream();
    } else {
        fileStream = downloadZip(files).body;
    }

    const reader = fileStream.getReader();

    const segmentPrefix = new Uint8Array(2);
    let filePlaintext = new Uint8Array();
    let segmentStart = 0;
    let segmentEnd = 0;
    // Start at 2 since we first encrypt the file name and mime type
    let count = 2;

    return new ReadableStream({
        async pull(controller) {
            // if there is no more file content, read more
            if (segmentEnd >= filePlaintext.length) {
                const { value, done } = await reader.read();
                if (done) {
                    controller.enqueue(new Uint8Array(2));
                    controller.close();
                    return;
                } else {
                    filePlaintext = value;
                    segmentStart = 0;
                    segmentEnd = 0;
                }
            }

            segmentEnd = Math.min(segmentStart + maxPlaintextSegmentSize, filePlaintext.length);

            const segmentPlaintext = filePlaintext.subarray(segmentStart, segmentEnd);
            const segmentCiphertext = await encrypt(key, count, segmentPlaintext);
            count++;

            segmentPrefix[0] = segmentCiphertext.byteLength / 256;
            segmentPrefix[1] = segmentCiphertext.byteLength % 256;

            controller.enqueue(segmentPrefix);
            controller.enqueue(segmentCiphertext);

            segmentStart += segmentPlaintext.length;
        }
    });
}

function updateProgress(socket, expectedBufferedAmount, id, obj, progressCallback) {
    let actualBufferedAmount = socket.bufferedAmount;
    let progress = expectedBufferedAmount - actualBufferedAmount;
    if (typeof progressCallback !== typeof undefined) {
        progressCallback(id, progress, obj);
    }
    return expectedBufferedAmount - progress;
}

async function readToSocket(
    socket, reader, progressTracker, id, obj, progressCallback)
{
    let expectedBufferedAmount = 0;

    while (socket.readyState == WebSocket.OPEN) {
        const { done, value } = await reader.read();
        if (done) {
            break;
        } else {
            // wait for the buffered amount to fall before enqueuing more data.
            let wait_ms = 1;
            while (socket.bufferedAmount > MAX_BUFFERED_AMOUNT) {
                await new Promise(r => setTimeout(r, wait_ms));
                if (wait_ms < MAX_SEND_WAIT_MS / 2) {
                    wait_ms *= 2;
                }
                expectedBufferedAmount = updateProgress(
                    socket, expectedBufferedAmount, id, obj, progressCallback);
            }

            expectedBufferedAmount += value.byteLength;

            socket.send(value);

            expectedBufferedAmount = updateProgress(
                socket, expectedBufferedAmount, id, obj, progressCallback);
        }
    }

    // after upload reading file finishes, there still may be enqueued data, so
    // keep updating the progress until that finishes
    while (socket.readyState != WebSocket.CLOSED && expectedBufferedAmount > 0) {
        await new Promise(r => setTimeout(r, 100));
        expectedBufferedAmount = updateProgress(
            socket, expectedBufferedAmount, id, obj, progressCallback);
    }

    progressTracker.uploadSucceeded = true;

    if (
        socket.readyState != WebSocket.CLOSING
        && socket.readyState != WebSocket.CLOSED)
    {
        socket.close();
    }
}

// Open a websocket connection over which a file will be uploaded. This function
// will handle the upload itself, but the websocket object is returned so that
// you can forcibly close it at any time.
//
// `url` is the address of the websocket endpoint
// `files` is an array of `File`
// `minutes` is the number of minutes before the upload expires
// `maxDownloads` is the number of downloads to permit before the upload expires
// `password` is the password required to download the file
//
// Set `maxDownloads` and `password` to `null` if they aren't to be used.
//
// The various callback parameters are called in response to changes in the
// progress of the upload.
//
// `obj` is an object that will be passed to each callback. It can be used to
// preserve some caller-defined state.
//
// `progressCallback` is called with 3 arguments:
//   - A string containing the ID of the upload
//   - The number of bytes just written (not in total)
//   - `obj`
//
// `completionCallback` is called with 2 argument:
//   - A string containing the ID of the upload
//   - `obj`
//
// `idCallback` is called with 3 arguments:
//   - A string containing the ID of the upload
//   - A string containing the cryptographic key for the upload
//   - The value of `maxDownloads` with which `upload` was called
//   - The value of `password` with which `upload` was called
//   - `obj`
//
// `errorCallback` is called with 2 arguments:
//  - The error event
//  - `obj`
//  - the error code
//
//  `closeCallback` is called with 2 arguments:
//  - The close event
//  - `obj`
//
// `idCallback` is called first. It is called after the server responds with the
// upload ID assigned to this websocket connection.
//
// `progressCallback` is called each time bytes are written over the websocket.
//
// `completionCallback` is called when an upload finishes successfully.
//
// `errorCallback` is called when the websocket raises an error or the server
// reports an error code back to the client.
//
// `closeCallback` is called when the websocket closes.
//
//  NOTE: the callbacks will ONLY be called if their respective events are fired
//  AFTER idCallback is triggered.
async function upload(
    url, files, minutes, maxDownloads, password, obj, progressCallback,
    completionCallback, idCallback, errorCallback, closeCallback)
{
    const key = await genKey();

    let nameBytes;
    let mimeBytes;
    let id = null;

    let progressTracker = {
        uploadSucceeded: false
    };

    if (files.length == 1) {
        nameBytes = textEncoder.encode(files[0].name);
        mimeBytes = textEncoder.encode(files[0].type);
    } else {
        nameBytes = new Uint8Array();
        mimeBytes = textEncoder.encode("application/zip");
    }

    let nameCipher = await encrypt(key, 0, nameBytes);
    let mimeCipher = await encrypt(key, 1, mimeBytes);

    const name = b64Encode(String.fromCharCode(...nameCipher));
    const mime = b64Encode(String.fromCharCode(...mimeCipher));

    url = url.concat("?file-name=", name);
    url = url.concat("&mime-type=", mime);
    url = url.concat("&minutes=", minutes.toString());

    if (typeof maxDownloads !== typeof undefined && maxDownloads != null) {
        url = url.concat("&max-downloads=", maxDownloads.toString());
    }

    if (typeof password !== typeof undefined && password != null) {
        url = url.concat("&password=", encodeURIComponent(password));
    }


    const socket = new WebSocket(url);
    socket.binaryType = "arraybuffer";

    if (typeof errorCallback !== typeof undefined) {
        socket.addEventListener('error', ev => {
            errorCallback(ev, obj, -1);
        });
    }

    if (typeof closeCallback !== typeof undefined) {
        socket.addEventListener('close', ev => {
            closeCallback(ev, obj);

            if (
                progressTracker.uploadSucceeded
                && socket.bufferedAmount == 0)
            {
                if (typeof completionCallback !== typeof undefined) {
                    completionCallback(id, obj);
                }
            } else if (typeof errorCallback !== typeof undefined) {
                errorCallback(ev, obj, -1);
            }
        });
    }

    const messageEventHandler = async e => {
        socket.removeEventListener('message', messageEventHandler);
        // if the server sends another message, it will be to report an error
        // code to the client.
        socket.addEventListener('message', msg => {
            if (typeof errorCallback !== typeof undefined) {
                const msgArr = new Uint8Array(msg.data);
                errorCallback(msg, obj, msgArr[0]);
            }
        });

        id = e.data;
        const encodedKey = await encodeKey(key);

        if (typeof idCallback !== typeof undefined) {
            idCallback(id, encodedKey, maxDownloads, password, obj);
        }

        const stream = await encryptStream(files, key);
        const reader = stream.getReader();

        await readToSocket(
            socket, reader, progressTracker, id, obj, progressCallback);
    };

    socket.addEventListener('message', messageEventHandler);

    return socket;
}


if (typeof window != typeof undefined) {
    window.transpoUpload = upload;
}

export { upload };
