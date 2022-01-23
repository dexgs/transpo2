// Return a human-readable string representing the given size in bytes
function sizeString(bytes) {
    let power = Math.floor(Math.log(bytes) / Math.log(10));
    let decimal = 0.0;
    let unit = "";
    if (power < 3) {
        decimal = bytes;
        unit = "B";
    } else if (power < 6) {
        decimal = bytes / Math.pow(10, 3);
        unit = "kB";
    } else if (power < 9) {
        decimal = bytes / Math.pow(10, 6);
        unit = "MB";
    } else if (power < 12) {
        decimal = bytes / Math.pow(10, 9);
        unit = "GB";
    } else {
        decimal = bytes / Math.pow(10, 12);
        unit = "TB";
    }

    decimal *= 100;
    decimal = Math.trunc(decimal);
    decimal /= 100;

    return decimal.toString().concat(unit);
}
