use std::{cmp, str};

const CD_PREFIX: &'static [u8] = b"Content-Disposition: ";
const CT_PREFIX: &'static [u8] = b"Content-Type: ";
const TERMINATOR: &'static [u8] = b"--"; // Come with me if you want to live.
const NEWLINE: &'static [u8] = b"\r\n";
// const DOUBLE_NEWLINE: &'static [u8] = b"\r\n\r\n";
const NEWLINE_BYTE_MAP: &'static [bool] = &newline_byte_map();

pub enum ParseResult<'a> {
    //       bytes  c-disp   c-type   value
    NewValue(usize, &'a str, &'a str, &'a [u8]),
    //       bytes  value
    Continue(usize, &'a [u8]),
    Finished,
    Error
}

// Returns the length of the data parsed, what was parsed and, if it is a new
// form field, the Content-Disposition (and Content-Type if it has one).
//
// Subsequent calls to this function MUST guarantee that `buf` begins where
// parsing last ended.
//
// `boundary` is assumed to be prefixed with "\r\n--"
pub fn parse<'a, B>(buf: &'a [u8], boundary: B, boundary_byte_map: &[bool]) -> ParseResult<'a>
where B: AsRef<[u8]>
{
    let boundary = boundary.as_ref();

    if let Some(buf) = buf.strip_prefix(boundary) {
        // This is either the end of the form or the start of a new form field
        if buf.starts_with(TERMINATOR) {
            // This is the end of the form
            ParseResult::Finished
        } else if let Some((buf, cd_len, has_ct, ct_len)) = buf.strip_prefix(NEWLINE)
            .and_then(|buf| buf.strip_prefix(CD_PREFIX))
            .and_then(|buf| Some((buf, find_subslice(buf, NEWLINE, NEWLINE_BYTE_MAP)?)))
            .and_then(|(buf, cd_len)| {
                let after_cd = &buf[(cd_len + NEWLINE.len())..];
                match after_cd.strip_prefix(CT_PREFIX) {
                    Some(after_cd) => Some((
                            buf, cd_len, true,
                            find_subslice(after_cd, NEWLINE, NEWLINE_BYTE_MAP)?)),
                    None => Some((buf, cd_len, false, 0))
                }
            })
        {
            // This is a new field in the form

            // New fields always have a Content-Disposition
            let cd_str = str::from_utf8(&buf[..cd_len]);
            let cd_total_len = CD_PREFIX.len() + cd_len + NEWLINE.len();

            // New fields do *not* always have a Content-Type
            //
            // We *DON'T* use ct_total_len here because it includes the length
            // of CD_PREFIX which is stripped off the value of `buf` in this
            // scope!
            let (ct_str, ct_total_len) = if has_ct {
                let ct_total_len = CT_PREFIX.len() + ct_len + NEWLINE.len();
                // Length of the contents of buf that come before the content type
                let before_len = cd_len + NEWLINE.len() + CT_PREFIX.len();
                (str::from_utf8(&buf[before_len..][..ct_len]), ct_total_len)
            } else {
                // Even if there's no Content-Type, there is still a blank line
                (Ok(""), NEWLINE.len())
            };

            match cd_str.and_then(|cd| ct_str.and_then(|ct| Ok((cd, ct)))) {
                Ok((cd, ct)) => {
                    let value = &buf[(cd_len + NEWLINE.len() + ct_total_len)..];
                    let value_len = find_value_len(value, boundary, boundary_byte_map);

                    let leading_len = boundary.len()
                        + NEWLINE.len()
                        + cd_total_len
                        + ct_total_len;

                    ParseResult::NewValue(leading_len + value_len, cd, ct, &value[..value_len])
                },
                Err(_) => ParseResult::Error
            }
        } else {
            // The form is improperly formatted
            ParseResult::Error
        }
    } else {
        // This is the continuation of the value of the previous field
        let value_len = find_value_len(buf, boundary, boundary_byte_map);
        ParseResult::Continue(value_len, &buf[..value_len])
    }
}

// Return the index of the first instance of s2 in s1
fn find_subslice(s1: &[u8], s2: &[u8], s2_byte_map: &[bool]) -> Option<usize>
{
    let mut i = 0;
    'outer: while i + s2.len() <= s1.len() {
        for j in (1..s2.len()).rev() {
            if !s2_byte_map[s1[i + j - 1] as usize] {
                i += j;
                continue 'outer;
            }
        }

        if &s1[i..(i + s2.len())] == s2 {
            return Some(i);
        } else {
            i += 1;
        }
    }

    None
}

// Return the index at which a subslice of s2 (must equal s2[0..n] for any n)
// occurs at the end of s1.
//
// Example: for s1 = "foobar" and s2 = "barnacle", the functioun should return 3
fn find_ending_subslice_of(s1: &[u8], s2: &[u8], s2_byte_map: &[bool]) -> Option<usize>
{
    if s1.len() > 1 && s2.len() > 1 {
        for sub_len in 1..=cmp::min(s2.len(), s1.len()) {
            if !s2_byte_map[s1[s1.len() - sub_len] as usize] {
                return None;
            } else if s1.ends_with(&s2[..sub_len]) {
                return Some(s1.len() - sub_len);
            }
        }
    }

    None
}

// Return the possible ending for the current value, either because the
// boundary is present in the current buffer, or a subslice of it is and it's
// possible that it will be completed on the next parse. If the value is not
// terminated within the contents of `buf`, the length of `buf` is returned.
fn find_value_len<B1, B2>(buf: B1, boundary: B2, boundary_byte_map: &[bool]) -> usize
where B1: AsRef<[u8]>,
      B2: AsRef<[u8]>
{
    let buf = buf.as_ref();
    let boundary = boundary.as_ref();

    if let Some(i) = find_subslice(buf, boundary, boundary_byte_map) {
        i
    } else if let Some(i) = find_ending_subslice_of(buf, boundary, boundary_byte_map) {
        i
    } else {
        buf.len()
    }
}

// Return an array of bool where the value at index n (for any n: u8) represents
// whether or not that byte is present in the given byte string
pub fn byte_map(s: &[u8]) -> [bool; u8::MAX as usize + 1] {
    let mut map = [false; u8::MAX as usize + 1];
    for b in s {
        map[*b as usize] = true;
    }
    map
}

// So we can have NEWLINE_BYTE_MAP as a constant
const fn newline_byte_map() -> [bool; u8::MAX as usize + 1] {
    let mut map = [false; u8::MAX as usize + 1];
    map[b'\r' as usize] = true;
    map[b'\n' as usize] = true;
    map
}


#[cfg(test)]
mod tests {
    use crate::multipart_form::*;

    #[test]
    fn test_find_subslice() {
        let s1 = vec![1, 2, 3, 3, 2, 5, 1];

        assert_eq!(find_subslice(&s1, &vec![3, 2], &byte_map(&vec![3, 2])), Some(3));
        assert_eq!(find_subslice(&s1, &vec![5, 5], &byte_map(&vec![5, 5])), None);
        assert_eq!(find_subslice(&s1, &vec![1], &byte_map(&vec![1])), Some(0));
        assert_eq!(find_subslice(&s1, &vec![0; 20], &byte_map(&vec![0; 20])), None);
        assert_eq!(find_subslice(&s1, &s1, &byte_map(&s1)), Some(0));
    }

    #[test]
    fn test_find_ending_subslice_of() {
        let s1 = b"foobar";

        assert_eq!(find_ending_subslice_of(s1, b"barnacle", &byte_map(b"barnacle")), Some(3));
        assert_eq!(find_ending_subslice_of(s1, b"foobar", &byte_map(b"foobar")), Some(0));
        assert_eq!(find_ending_subslice_of(s1, b"foo", &byte_map(b"foo")), None);
    }

    #[test]
    fn test_parse() {
        const FORM_BODY: &'static [u8] =
b"\r
--boundary\r
Content-Disposition: form-data; name=\"field1\"\r
\r
value1\r
--boundary\r
Content-Disposition: form-data; name=\"field2\"; filename=\"example.txt\"\r
\r
value2\r
--boundary--";

        const BOUNDARY: &'static [u8] = b"\r\n--boundary";
        let byte_map = byte_map(BOUNDARY);
        const CD_1: &'static str = "form-data; name=\"field1\"";
        const VALUE_1: &'static [u8] = b"value1";
        const CD_2: &'static str = "form-data; name=\"field2\"; filename=\"example.txt\"";
        const VALUE_2: &'static [u8] = b"value2";

        let mut i = 0;
        let mut value = 0;
        loop {
            match parse(&FORM_BODY[i..], BOUNDARY, &byte_map) {
                ParseResult::NewValue(len, cd, ct, val) => {
                    i += len;

                    if value == 0 {
                        assert_eq!(cd, CD_1);
                        assert_eq!(val, VALUE_1);
                    } else {
                        assert_eq!(cd, CD_2);
                        assert_eq!(val, VALUE_2);
                    }

                    value += 1;
                },
                ParseResult::Continue(len, _val) => i += len,
                ParseResult::Finished | ParseResult::Error => break
            }
        }
    }
}
