const ENCRYPT_PARAMS = {
    name: "AES-GCM",
    iv: new Uint8Array(12),
    tagLength: 128
};

const DECRYPT_PARAMS = {
    name: "AES-GCM",
    iv: new Uint8Array(12),
};

// Maximum length of ciphertext to be decrypted at once
const maxSegmentSize = 10240 + 16;


function stringToBytes(string) {
    const bytes = new Uint8Array(string.length);
    for (let i = 0; i < bytes.length; i++) {
        bytes[i] = string.charCodeAt(i);
    }
    return bytes;
}

function b64ToUrlSafe(b64) {
    b64 = b64.replaceAll("+", "-");
    b64 = b64.replaceAll("/", "_");
    b64 = b64.replaceAll("=", "");

    return b64;
}

// URL-safe base64 encode
function b64Encode(str) {
    let b64 = btoa(str);
    return b64ToUrlSafe(b64);
}

function b64FromUrlSafe(b64) {
    // convert back to b64 the browser can decode
    b64 = b64.replaceAll("-", "+");
    b64 = b64.replaceAll("_", "/");

    let padding = 4 - (b64.length % 4);
    if (padding == 2) {
        b64 = b64.concat("==")
    } else if (padding == 1) {
        b64 = b64.concat("=")
    }

    return b64;
}

// URL-safe base64 decode
function b64Decode(b64) {
    b64 = b64FromUrlSafe(b64);
    return atob(b64);
}

// Generate a key for AES256-GCM
async function genKey() {
    let params = {
        name: "AES-GCM",
        length: 256,
    };

    return await crypto.subtle.generateKey(params, true, ["encrypt", "decrypt"]);
}

// Base64 encode a key
async function encodeKey(key) {
    let bytes = await crypto.subtle.exportKey("raw", key);
    return b64Encode(String.fromCharCode(...new Uint8Array(bytes)));
}

// Decode a base64 encoded key
async function decodeKey(b64) {
    let decoded = b64Decode(b64);

    let bytes = new Uint8Array(decoded.length);
    for (let i = 0; i < bytes.length; i++) {
        bytes[i] = decoded.charCodeAt(i);
    }

    return await crypto.subtle.importKey("raw", bytes, "AES-GCM", true, ["encrypt", "decrypt"]);
}

// Encrypt plaintext with the given key
async function encrypt(key, plaintext) {
    return new Uint8Array(await crypto.subtle.encrypt(ENCRYPT_PARAMS, key, plaintext));
}

// Decrypt ciphertext using the given key
async function decrypt(key, ciphertext) {
    return new Uint8Array(await crypto.subtle.decrypt(DECRYPT_PARAMS, key, ciphertext.buffer));
}

export { maxSegmentSize, genKey, b64Decode, b64Encode, stringToBytes, encodeKey, decodeKey, encrypt, decrypt };
