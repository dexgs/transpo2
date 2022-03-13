// Return a human-readable string representing the given size in bytes
function sizeString(bytes) {
    if (bytes == 0) {
        return "0B";
    }

    let power = Math.floor(Math.log(bytes) / Math.log(1000));
    let decimal = 0.0;
    let unit = "";

    switch (power) {
        case 0:
            unit = "B"; break;
        case 1:
            unit = "kB"; break;
        case 2:
            unit = "MB"; break;
        case 3:
            unit = "GB"; break; 
        default:
            unit = "TB";
    }

    decimal = bytes / Math.pow(1000, power);

    decimal *= 100;
    decimal = Math.trunc(decimal);
    decimal /= 100;

    return decimal.toString().concat(unit);
}
