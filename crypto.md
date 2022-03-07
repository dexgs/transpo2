# POST upload

A client-side encrypted upload should be structured as follows:

Transpo uses 256-bit AES-GCM encryption. The nonce used during encryption and
decryption has all 12 bytes set to 0 because each key is only one time.

The file name should be encrypted first, then the mime type should be encrypted.
Both the encrypted file name and encrypted mime type should then be base-64
encoded. To make the base64-encoded ciphertexts URL-safe, `+` is replaced with
`-` and `/` is replaced with `_`. The file name and mime type are to be
encrypted in this order BEFORE any file contents are encrypted.

The upload should be a POST request with multipart encoding and a form boundary
no longer than 70 bytes. The fields of the form are as follows:

* `server-side-processing` (`on` or `off`) (optional)
* `enable-multiple-files` (`on` or `off`) (optional)
* `files` (`file contents`)
* `days` (`int`)
* `hours` (`int`)
* `minutes` (`int`)
* `enable-max-downloads` (`on` or `off`) (optional)
* `max-downloads` (`int`) (optional)
* `enable-password` (`on` or `off`) (optional)
* `password` (`text`) (optional)

If `server-side-processing` is set to `on`, it MUST be sent BEFORE any file
contents.

if `enable-multiple-files` is set to `on`, it MUST be send BEFORE any file
contents.

The `name` field in the `Content-Disposition` header for `files` should be the
URL safe base64-encoded file name ciphertext.

The value of the `Content-Type` header for `files` should be the URL safe
base64-encoded mime type ciphertext.

The contents of `files` should be broken up into segments no longer than 10256
bytes. Each segment should be prefixed by 2 bytes storing a 16-bit unsigned
integer in big-endian byte order. This integer contains the number of bytes
in the following segment (without counting its own two bytes). The contents of
`files` should be terminated by two bytes each equal to zero.

The contents of each length-prefixed segment in `files` should the ciphertext
result of encrypting a segment of the upload. The segments should be produced by
first encrypting a segment of the upload, writing the length of the ciphertext
as an unsigned 16-bit integer in big-endian byte order to the form body, and
then writing the ciphertext itself.

# WebSocket upload

A client-side encrypted upload can also be made over a WebSocket connection.

The following values are to be sent to the server via the query string in the
path at which the WebSocket connection is opened

* `minutes` (`int`)
* `password` (`text`) (optional)
* `download_limit` (`int`) (optional)
* `file-name` (`text`)
* `mime-type` (`text`)

`file-name` and `mime-type` are to be base64-encoded ciphertexts as described in
the first section.

The body of the file is encrypted the same way as is described in the first
section with the difference that it is transfered over the WebSocket connection
instead of in the body of a form.

Unlike when an upload is made using a POST request, uploads made over WebSocket
connections MUST be encrypted client-side.

See also:
 * [multipart POST](https://wdeveloper.mozilla.org/en-US/docs/Web/HTTP/Methods/POST#example)
