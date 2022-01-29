use std::collections::HashMap;

// '+' becomes '-' and '/' becomes '_' for URL safety
const BASE64_TABLE: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O',
    'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd',
    'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7',
    '8', '9', '-', '_'
];

// '=' becomes '.' for URL safety
const BASE64_PADDING: char = '.';
const BASE64_PADDING_BYTE: u8 = u8::MAX;

const fn map_b64(digit: char) -> u8 {
    match digit {
        'A' => 0, 'B' => 1, 'C' => 2, 'D' => 3, 'E' => 4, 'F' => 5, 'G' => 6,
        'H' => 7, 'I' => 8, 'J' => 9, 'K' => 10, 'L' => 11, 'M' => 12,
        'N' => 13, 'O' => 14, 'P' => 15, 'Q' => 16, 'R' => 17, 'S' => 18,
        'T' => 19, 'U' => 20, 'V' => 21, 'W' => 22, 'X' => 23, 'Y' => 24,
        'Z' => 25, 'a' => 26, 'b' => 27, 'c' => 28, 'd' => 29, 'e' => 30,
        'f' => 31, 'g' => 32, 'h' => 33, 'i' => 34, 'j' => 35, 'k' => 36,
        'l' => 37, 'm' => 38, 'n' => 39, 'o' => 40, 'p' => 41, 'q' => 42,
        'r' => 43, 's' => 44, 't' => 45, 'u' => 46, 'v' => 47, 'w' => 48,
        'x' => 49, 'y' => 50, 'z' => 51, '0' => 52, '1' => 53, '2' => 54,
        '3' => 55, '4' => 56, '5' => 57, '6' => 58, '7' => 59, '8' => 60,
        '9' => 61, '-' => 62, '_' => 63,
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

        let b64_1 = map_b64(b64[i] as char);
        let b64_2 = map_b64(b64[i + 1] as char);
        let b64_3 = map_b64(b64[i + 2] as char);
        let b64_4 = map_b64(b64[i + 3] as char);

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
