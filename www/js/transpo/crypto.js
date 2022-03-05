var transpoCrypto = (() => {
    const PARAMS = {
        "name": "AES-GCM",
        "iv": new Uint8Array(12)
    };

    var publicAPI = {};

    // URL-safe base64 encode
    publicAPI.b64encode = function(str) {
        let b64 = btoa(str);

        b64 = b64.replaceAll("+", "-");
        b64 = b64.replaceAll("/", "_");
        b64 = b64.replaceAll("=", "");

        return b64;
    };

    // URL-safe base64 decode
    publicAPI.b64decode = function(b64) {
        // convert back to b64 the browser can decode
        b64 = b64.replaceAll("-", "+");
        b64 = b64.replaceAll("_", "/");

        let padding = 4 - (b64.length % 4);
        if (padding == 2) {
            b64 = b64.concat("==")
        } else if (padding == 1) {
            b64 = b64.concat("=")
        }

        return atob(b64);
    };

    // Generate a key for AES256-GCM
    publicAPI.genKey = async function() {
        let params = {
            "name": "AES-GCM",
            "length": 256
        };

        return await crypto.subtle.generateKey(params, true, ["encrypt", "decrypt"]);
    };

    // Base64 encode a key
    publicAPI.encodeKey = async function(key) {
        let bytes = await crypto.subtle.exportKey("raw", key);
        return publicAPI.b64encode(String.fromCharCode(...new Uint8Array(bytes)));
    };

    // Decode a base64 encoded key
    publicAPI.decodeKey = async function(b64) {
        let decoded = publicAPI.b64decode(b64);

        let bytes = new Uint8Array(decoded.length);
        for (let i = 0; i < bytes.length; i++) {
            bytes[i] = decoded.charCodeAt(i);
        }

        return await crypto.subtle.importKey("raw", bytes, "AES-GCM", false, ["encrypt", "decrypt"]);
    };

    // Encrypt plaintext with the given key
    publicAPI.encrypt = async function(key, plaintext) {
        return await crypto.subtle.encrypt(PARAMS, key, plaintext);
    };

    // Decrypt ciphertext using the given key
    publicAPI.decrypt = async function(key, ciphertext) {
        return await crypto.subtle.decrypt(PARAMS, key, ciphertext);
    };

    return publicAPI;
})();
