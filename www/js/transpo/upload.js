import { maxPlaintextSegmentSize, b64Encode, encrypt, genKey, encodeKey } from "./crypto.js";
import { downloadZip } from "./client-zip/index.js";

const textEncoder = new TextEncoder("utf-8");


async function encryptStream(files, key, id, obj, progressCallback, completionCallback) {
    let fileStream;
    if (files.length == 1) {
        fileStream = files[0].stream();
    } else {
        fileStream = downloadZip(files).body;
    }

    const reader = fileStream.getReader();

    let filePlaintext = new Uint8Array();
    let segmentStart = 0;
    let segmentEnd = 0;

    return new ReadableStream({
        async pull(controller) {
            // if there is no more file content, read more
            if (segmentEnd >= filePlaintext.length) {
                const { value, done } = await reader.read();
                if (done) {
                    controller.enqueue(new Uint8Array(2));
                    controller.close();

                    if (typeof completionCallback !== typeof undefined) {
                        completionCallback(id, obj);
                    }

                    return;
                } else {
                    filePlaintext = value;
                    segmentStart = 0;
                    segmentEnd = 0;
                }
            }

            segmentEnd = Math.min(segmentStart + maxPlaintextSegmentSize, filePlaintext.length);

            const segmentPlaintext = filePlaintext.subarray(segmentStart, segmentEnd);
            const segmentCiphertext = await encrypt(key, segmentPlaintext);
            const segmentPrefix = new Uint8Array(2);
            segmentPrefix[0] = segmentCiphertext.byteLength / 256;
            segmentPrefix[1] = segmentCiphertext.byteLength % 256;

            controller.enqueue(segmentPrefix);
            controller.enqueue(segmentCiphertext);

            if (typeof progressCallback !== typeof undefined) {
                progressCallback(id, segmentPlaintext.length, obj);
            }

            segmentStart += segmentPlaintext.length;
        }
    });
}

async function readToSocket(socket, reader) {
    for (let i = 0; i < 100; i++) {
        const { done, value } = await reader.read();
        if (done) {
            socket.close();
            return;
        } else if (socket.readyState == 1) {
            socket.send(value.buffer);
        }
    }

    setTimeout(async () => { await readToSocket(socket, reader) }, 0);
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
// `errorCallback` is called with 3 arguments:
//  - A string containing the ID of the upload
//  - The error event
//  - `obj`
//
//  `closeCallback` is called with 2 arguments:
//  - A string containing the ID of the upload
//  - `obj`
//
// `idCallback` is called first. It is called after the server responds with the
// upload ID assigned to this websocket connection.
//
// `progressCallback` is called each time bytes are written over the websocket.
//
// `completionCallback` is called when an upload finishes successfully.
//
// `errorCallback` is called wthen an error occurs during an upload.
//
// `closeCallback` is called when the socket closes.
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

    if (files.length == 1) {
        nameBytes = textEncoder.encode(files[0].name);
        mimeBytes = textEncoder.encode(files[0].type);
    } else {
        nameBytes = new Uint8Array();
        mimeBytes = textEncoder.encode("application/zip");
    }

    let nameCipher = await encrypt(key, nameBytes);
    let mimeCipher = await encrypt(key, mimeBytes);

    const name = b64Encode(String.fromCharCode(...nameCipher));
    const mime = b64Encode(String.fromCharCode(...mimeCipher));

    url = url.concat("?file-name=", name);
    url = url.concat("&mime-type=", mime);
    url = url.concat("&minutes=", minutes.toString());

    if (typeof maxDownloads !== typeof undefined && maxDownloads != null) {
        url = url.concat("&max-downloads=", maxDownloads.toString());
    }

    if (typeof downloadLimit !== typeof undefined && password != null) {
        url = url.concat("&password=", encodeURIComponent(password));
    }

    const messageEventHandler = async e => {
        socket.removeEventListener('message', messageEventHandler);

        const id = e.data;
        const encodedKey = await encodeKey(key);


        if (typeof errorCallback !== typeof undefined) {
            socket.addEventListener('error', ev => {
                errorCallback(id, ev, obj);
            });
        }


        if (typeof closeCallback !== typeof undefined) {
            socket.addEventListener('close', () => {
                closeCallback(id, obj);
            });
        }

        if (typeof idCallback !== typeof undefined) {
            idCallback(id, encodedKey, maxDownloads, password, obj);
        }

        const stream = await encryptStream(
            files, key, id, obj,
            progressCallback, completionCallback);
        const reader = stream.getReader();

        await readToSocket(socket, reader);
    };

    const socket = new WebSocket(url);

    socket.binaryType = "arraybuffer";
    socket.addEventListener('message', messageEventHandler);

    return socket;
}


if (typeof window != typeof undefined) {
    window.transpoUpload = upload;
}

export { upload };
