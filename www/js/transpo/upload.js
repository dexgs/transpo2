var transpoUploader = (() => {
    var publicAPI = {};

    var singleFileReadableStream = async function(file) {
    };

    var multiFileReadableStream = async function(files) {
    };

    var getNameAndMime = function(files, key) {
        if (files.length == 0) {
            let name_bytes = new TextEncoder("utf-8").encode(files[0].name);
            let mime_bytes = new TextEncoder("utf-8").encode(files[0].type);

            let name_cipher = transpoCrypto.encrypt(key, name_bytes);
            let mime_cipher = transpoCrypto.encrypt(key, mime_bytes);

            return {
                "name": transpoCrypto.b64encode(String.fromCharCode(...name_cipher)),
                "mime": transpoCrypto.b64encode(String.fromCharCode(...mime_cipher))
            };
        } else {
        }
    };

    publicAPI.upload = async function(files, form, url, key, progressCallback, completionCallback) {
    };

    return publicAPI;
})();
