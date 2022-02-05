use std::collections::HashMap;

// '+' becomes '-' and '/' becomes b'_' for URL safety
pub const BASE64_TABLE: &[u8] = &[
    b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L',
    b'M', b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X',
    b'Y', b'Z', b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j',
    b'k', b'l', b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v',
    b'w', b'x', b'y', b'z', b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7',
    b'8', b'9', b'-', b'_'
];

// '=' becomes '.' for URL safety
const BASE64_PADDING: u8 = b'.';
const BASE64_PADDING_BYTE: u8 = u8::MAX;

const fn map_b64(digit: u8) -> u8 {
    match digit {
        b'A' => 0, b'B' => 1, b'C' => 2, b'D' => 3, b'E' => 4, b'F' => 5, b'G' => 6,
        b'H' => 7, b'I' => 8, b'J' => 9, b'K' => 10, b'L' => 11, b'M' => 12,
        b'N' => 13, b'O' => 14, b'P' => 15, b'Q' => 16, b'R' => 17, b'S' => 18,
        b'T' => 19, b'U' => 20, b'V' => 21, b'W' => 22, b'X' => 23, b'Y' => 24,
        b'Z' => 25, b'a' => 26, b'b' => 27, b'c' => 28, b'd' => 29, b'e' => 30,
        b'f' => 31, b'g' => 32, b'h' => 33, b'i' => 34, b'j' => 35, b'k' => 36,
        b'l' => 37, b'm' => 38, b'n' => 39, b'o' => 40, b'p' => 41, b'q' => 42,
        b'r' => 43, b's' => 44, b't' => 45, b'u' => 46, b'v' => 47, b'w' => 48,
        b'x' => 49, b'y' => 50, b'z' => 51, b'0' => 52, b'1' => 53, b'2' => 54,
        b'3' => 55, b'4' => 56, b'5' => 57, b'6' => 58, b'7' => 59, b'8' => 60,
        b'9' => 61, b'-' => 62, b'_' => 63,
        _ => BASE64_PADDING_BYTE
    }
}

// encode the input bytes into URL-safe base64
pub fn base64_encode(bytes: &[u8]) -> Vec<u8> {
    let len = (bytes.len() + 2) / 3;
    let mut vec = Vec::with_capacity(len * 4);

    for i in 0..len {
        let i = i * 3;

        let first = bytes[i] >> 2;
        vec.push(BASE64_TABLE[first as usize] as u8);

        let mut second = bytes[i] << 4 & 0b00110000;
        if i + 1 < bytes.len() {
            second += bytes[i + 1] >> 4 & 0b00001111;
        }
        vec.push(BASE64_TABLE[second as usize] as u8);

        let mut third = bytes.get(i + 1).map(|b| b << 2 & 0b00111100);
        if let Some(b) = bytes.get(i + 2) {
            *third.as_mut().unwrap() += b >> 6 & 0b00000011;
        }
        match third {
            Some(third) => vec.push(BASE64_TABLE[third as usize] as u8),
            None => vec.push(BASE64_PADDING as u8)
        }

        let fourth = bytes.get(i + 2).map(|b| b & 0b00111111);
        match fourth {
            Some(fourth) => vec.push(BASE64_TABLE[fourth as usize] as u8),
            None => vec.push(BASE64_PADDING as u8)
        }
    }

    vec
}

// decode the input bytes from URL-safe base64 into an unencoded form
pub fn base64_decode(b64: &[u8]) -> Vec<u8> {
    let len = b64.len() / 4;
    let mut vec = Vec::with_capacity(len * 3);

    for i in 0..len {
        let i = i * 4;

        let b64_1 = map_b64(b64[i]);
        let b64_2 = map_b64(b64[i + 1]);
        let b64_3 = map_b64(b64[i + 2]);
        let b64_4 = map_b64(b64[i + 3]);

        let first = (b64_1 << 2) + (b64_2 >> 4);
        vec.push(first);

        let mut second = b64_2 << 4;
        if b64_3 != BASE64_PADDING_BYTE {
            second += b64_3 >> 2;
        }
        vec.push(second);

        if b64_4 != BASE64_PADDING_BYTE {
            let third = (b64_3 << 6) + b64_4;
            vec.push(third);
        }
    }

    vec
}


#[cfg(test)]
mod tests {
    use crate::b64::*;

    #[test]
    fn test_base64_encode() {
        let msg = "bazinga!";
        let expected_b64 = "YmF6aW5nYSE.";

        let b64 = base64_encode(msg.as_bytes());

        assert_eq!(expected_b64.as_bytes(), b64);
    }

    #[test]
    fn test_base64_decode() {
        let b64 = "YmF6aW5nYSE.";
        let expected_msg = "bazinga!";

        let msg = base64_decode(b64.as_bytes());

        assert_eq!(expected_msg.as_bytes(), msg);
    }
}
