const fileSizeError = document.getElementById("file-size-error");
const quotaError = document.getElementById("quota-error");
const networkError = document.getElementById("network-error");
const serverError = document.getElementById("server-error");
const protocolError = document.getElementById("protocol-error");
const unknownError = document.getElementById("unknown-error");


function showError(err) {
    switch (err) {
        case "-1":
            networkError.showModal();
            break;
        case "0":
            // other
            serverError.showModal();
            break;
        case "1":
            // file size
            fileSizeError.showModal();
            break;
        case "2":
            // quota
            quotaError.showModal();
            break;
        case "3":
            // storage
            serverError.showModal();
            break;
        case "4":
            // protocol
            protocolError.showModal();
            break;
        default:
            unknownError.showModal();
            break;
    }
}
