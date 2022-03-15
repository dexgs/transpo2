// '+' becomes '-' and '/' becomes b'_' for URL safety
pub const BASE64_TABLE: &[u8] = &[
    b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L',
    b'M', b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X',
    b'Y', b'Z', b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j',
    b'k', b'l', b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v',
    b'w', b'x', b'y', b'z', b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7',
    b'8', b'9', b'-', b'_'
];

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
        _ => u8::MAX
    }
}

// Return the number of bytes required to store the base64-encoded form of a
// byte string with length = `num_bytes`
pub const fn base64_encode_length(num_bytes: usize) -> usize {
    (num_bytes / 3) * 4 + match num_bytes % 3 {
        0 => 0,
        1 => 2,
        2 => 3,
        // This branch is impossible to reach
        _ => 0
    }
}

// Return the number of bytes required to store the base64-decoded form of
// a base64-encoded byte string with length = `num_bytes`
pub const fn base64_decode_length(num_bytes: usize) -> Option<usize> {
    Some((num_bytes / 4) * 3 + match num_bytes % 4 {
        0 => 0,
        1 => { return None }
        2 => 1,
        3 => 2,
        // This branch is impossible to reach
        _ => { return None }
    })
}

// encode the input bytes into URL-safe base64
pub fn base64_encode(bytes: &[u8]) -> Vec<u8> {
    let mut vec = Vec::with_capacity(base64_encode_length(bytes.len()));

    let len = (bytes.len() + 2) / 3;

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
            None => {}
        }

        let fourth = bytes.get(i + 2).map(|b| b & 0b00111111);
        match fourth {
            Some(fourth) => vec.push(BASE64_TABLE[fourth as usize] as u8),
            None => {}
        }
    }

    vec
}

// decode the input bytes from URL-safe base64 into an unencoded form
pub fn base64_decode(b64: &[u8]) -> Option<Vec<u8>> {
    let mut vec = Vec::with_capacity(base64_decode_length(b64.len())?);

    let len = (b64.len() + 3) / 4;

    for i in 0..len {
        let i = i * 4;

        let b64_1 = map_b64(*b64.get(i)?);
        let b64_2 = map_b64(*b64.get(i + 1)?);
        let b64_3 = b64.get(i + 2).map(|b| map_b64(*b));
        let b64_4 = b64.get(i + 3).map(|b| map_b64(*b));

        let first = (b64_1 << 2) + (b64_2 >> 4);
        vec.push(first);

        if let Some(b64_3) = b64_3 {
            let second = (b64_2 << 4) + (b64_3 >> 2); 
            vec.push(second);
        }

        if let Some(b64_4) = b64_4 {
            let third = (b64_3.unwrap() << 6) + b64_4;
            vec.push(third);
        }
    }

    Some(vec)
}

pub fn i64_to_b64_bytes(i: i64) -> Vec<u8> {
    let bytes = i.to_be_bytes();
    base64_encode(&bytes)
}

pub fn i64_from_b64_bytes(bytes: &[u8]) -> Option<i64> {
    let bytes = base64_decode(bytes)?;
    let mut i64_bytes = [0; 8];
    if bytes.len() >= 8 {
        i64_bytes.copy_from_slice(&bytes[..8]);
        Some(i64::from_be_bytes(i64_bytes))
    } else {
        None
    }
}


#[cfg(test)]
mod tests {
    use crate::b64::*;

    #[test]
    fn test_base64_encode() {
        let msg = "a simple test";
        let expected_b64 = "YSBzaW1wbGUgdGVzdA";

        let b64 = base64_encode(msg.as_bytes());

        assert_eq!(expected_b64.as_bytes(), b64);
    }

    #[test]
    fn test_base64_decode() {
        let b64 = "YSBzaW1wbGUgdGVzdA";
        let expected_msg = "a simple test";

        let msg = base64_decode(b64.as_bytes()).unwrap();

        assert_eq!(expected_msg.as_bytes(), msg);
    }
}
