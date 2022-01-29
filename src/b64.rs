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

// Return a string representing the given byte slice in base 64 (URL safe)
pub fn base64_encode(bytes: &[u8]) -> Vec<u8> {
    let len = (bytes.len() + 2) / 3;
    let mut vec = Vec::with_capacity(len * 4);

    for i in 0..len {
        let i = i * 3;

        let first = bytes[i] >> 2;

        let mut second = bytes[i] << 4 & 0b00110000;
        if i + 1 < bytes.len() {
            second += bytes[i + 1] >> 4 & 0b00001111;
        }

        let mut third = bytes.get(i + 1).map(|b| b << 2 & 0b00111100);
        if let Some(b) = bytes.get(i + 2) {
            *third.as_mut().unwrap() += b >> 6 & 0b00000011;
        }

        let fourth = bytes.get(i + 2).map(|b| b & 0b00111111);


        vec.push(BASE64_TABLE[first as usize] as u8);

        vec.push(BASE64_TABLE[second as usize] as u8);

        match third {
            Some(third) => vec.push(BASE64_TABLE[third as usize] as u8),
            None => vec.push(BASE64_PADDING as u8)
        }

        match fourth {
            Some(fourth) => vec.push(BASE64_TABLE[fourth as usize] as u8),
            None => vec.push(BASE64_PADDING as u8)
        }
    }

    vec
}

pub fn base64_decode(b64: &[u8]) -> Vec<u8> {
    Vec::new()
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
}
