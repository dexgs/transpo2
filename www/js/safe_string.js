// Sanitize a string before inserting it into the document
function safeString(str) {
    if (typeof str == typeof "") {
        return str
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;')
            .replace(/'/g, '&#39;');
    } else {
        return str;
    }
}
