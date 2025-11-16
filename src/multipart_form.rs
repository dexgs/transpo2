// https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Disposition
// https://developer.mozilla.org/en-US/docs/Web/HTTP/Methods/POST

use std::{cmp, str};

const CD_PREFIX: &'static [u8] = b"Content-Disposition: ";
const CD_PREFIX_BYTE_MAP: &'static [u8] = &byte_map(CD_PREFIX).unwrap();
const CT_PREFIX: &'static [u8] = b"Content-Type: ";
const CT_PREFIX_BYTE_MAP: &'static [u8] = &byte_map(CT_PREFIX).unwrap();
const TERMINATOR: &'static [u8] = b"--"; // Come with me if you want to live.
const NEWLINE: &'static [u8] = b"\r\n";
const NEWLINE_BYTE_MAP: &'static [u8] = &byte_map(NEWLINE).unwrap();

pub enum ParseResult<'a> {
    // There is a separate `bytes` field because the number of bytes parsed can
    // differ from the size of the `value` that gets returned because of the
    // additional leading data which prefixes the actual value.
    //
    //       bytes  c-disp   c-type   value
    NewValue(usize, &'a str, &'a str, &'a [u8]),
    //       value
    Continue(&'a [u8]),
    NeedMoreData,
    Finished,
    Error
}

// This is a stateless parser for multipart POST requests.
//
// Returns the length of the data parsed, what was parsed and, if it is a new
// form field, the Content-Disposition (and Content-Type if it has one).
//
// Subsequent calls to this function MUST guarantee that `buf` begins where
// parsing last stopped, i.e. the elements of buf starting at the index where
// parsing last stopped should be copied to the beginning of buf.
//
// Parsing is "stopped" when a conclusive result is returned (`NewValue` or
// `Continue`). If `NeedMoreData` is returned, more data should be read, but
// this data should be appended to the value passed as `buf` as the previous
// parse attempt required more data in order to return a conclusive result.
//
// Parsing is finished when `Finished` or `Error` is returned.
//
// `boundary` MUST begin with "\r\n--"
pub fn parse<'a, B>(buf: &'a [u8], boundary: B, boundary_byte_map: &[u8]) -> ParseResult<'a>
where B: AsRef<[u8]>
{
    let boundary = boundary.as_ref();

    if let Some(buf) = buf.strip_prefix(boundary) {
        // This is either the end of the form or the start of a new form field
        if buf.starts_with(TERMINATOR) {
            // This is the end of the form
            ParseResult::Finished
        } else {
            // Extract the content-disposition and content-type from the value,
            // or return early if the form is malformed or potentially cut off
            // by the end of the buffer, requiring another read.
            let parse_result = try_strip_prefix(buf, NEWLINE, NEWLINE_BYTE_MAP)
                .and_then(|buf| try_strip_prefix(buf, CD_PREFIX, CD_PREFIX_BYTE_MAP))
                .and_then(|buf| Ok((buf, try_find_subslice(buf, NEWLINE, NEWLINE_BYTE_MAP)?)))
                .and_then(|(buf, cd_len)| {
                    let after_cd = &buf[(cd_len + NEWLINE.len())..];
                    match try_strip_prefix(after_cd, CT_PREFIX, CT_PREFIX_BYTE_MAP) {
                        Err(ParseResult::NeedMoreData) => {
                            // There is possibly an incomplete Content-Type prefix
                            return Err(ParseResult::NeedMoreData)
                        },
                        Err(ParseResult::Error) => {
                            // There is no Content-Type
                            Ok((buf, cd_len, false, 0))
                        },
                        Ok(after_cd) => {
                            // There is a Content-Type
                            let ct_len = try_find_subslice(after_cd, NEWLINE, NEWLINE_BYTE_MAP)?;
                            Ok((buf, cd_len, true, ct_len))
                        }
                        // This case will not happen (see `try_strip_prefix`).
                        // It is only here to appease the compiler.
                        _ => Err(ParseResult::Error)
                    }});

            let (buf, cd_len, has_ct, ct_len) = match parse_result {
                Ok(values) => values,
                Err(result) => return result
            };

            // This is a new field in the form

            // New fields always have a Content-Disposition
            let cd_str = match str::from_utf8(&buf[..cd_len]) {
                Ok(cd_str) => cd_str,
                Err(_) => return ParseResult::Error
            };
            let cd_total_len = CD_PREFIX.len() + cd_len + NEWLINE.len();

            // New fields do *not* always have a Content-Type
            //
            // We *DON'T* use cd_total_len here because it includes the length
            // of CD_PREFIX which is stripped off the value of `buf` in this
            // scope!
            let (ct_str, ct_total_len) = if has_ct {
                let ct_total_len = CT_PREFIX.len() + ct_len + 2 * NEWLINE.len();
                // Length of the contents of buf that come before the content type
                let before_len = cd_len + NEWLINE.len() + CT_PREFIX.len();
                let ct_str = match str::from_utf8(&buf[before_len..][..ct_len]) {
                    Ok(ct_str) => ct_str,
                    Err(_) => return ParseResult::Error
                };
                (ct_str, ct_total_len)
            } else {
                // When there is no Content-Type, there is still a blank line
                ("", NEWLINE.len())
            };

            let value_start_index = cd_len + NEWLINE.len() + ct_total_len;
            if value_start_index < buf.len() {
                let value = &buf[(cd_len + NEWLINE.len() + ct_total_len)..];
                let value_len = find_value_len(value, boundary, boundary_byte_map);

                let leading_len = boundary.len()
                    + NEWLINE.len()
                    + cd_total_len
                    + ct_total_len;

                ParseResult::NewValue(
                    leading_len + value_len,
                    cd_str, ct_str,
                    &value[..value_len])
            } else {
                ParseResult::Error
            }
        }
    } else {
        // This is the continuation of the value of the previous field
        let value_len = find_value_len(buf, boundary, boundary_byte_map);
        ParseResult::Continue(&buf[..value_len])
    }
}

// Strip the given prefix off of buf. If buf does not start with the given
// prefix, return a parse result of either NeedMoreData if buf could possibly
// start with prefix if it were longer or Error if it does not and cannot
fn try_strip_prefix<'a>(buf: &'a [u8], prefix: &[u8], prefix_byte_map: &[u8]) -> Result<&'a [u8], ParseResult<'a>> {
    match buf.strip_prefix(prefix) {
        Some(buf) => Ok(buf),
        None => {
            if buf.len() < prefix.len()
                && ends_with_subslice(buf, prefix, prefix_byte_map)
            {
                Err(ParseResult::NeedMoreData)
            } else {
                Err(ParseResult::Error)
            }
        }
    }
}

fn try_find_subslice<'a>(buf: &'a [u8], prefix: &[u8], prefix_byte_map: &[u8]) -> Result<usize, ParseResult<'a>> {
    match find_subslice(buf, prefix, prefix_byte_map) {
        Some(index) => Ok(index),
        None => Err(ParseResult::NeedMoreData)
    }
}

// Return the index of the first instance of s2 in s1
fn find_subslice(s1: &[u8], s2: &[u8], s2_byte_map: &[u8]) -> Option<usize>
{
    let mut i = 0;

    while i + s2.len() <= s1.len() {
        let skip = s2_byte_map[s1[i + s2.len() - 1] as usize];
        if skip > 0 {
            i += skip as usize;
        } else if &s1[i..(i + s2.len())] == s2 {
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
fn find_ending_subslice_of(s1: &[u8], s2: &[u8], s2_byte_map: &[u8]) -> Option<usize>
{
    if s1.len() == 0 || s2.len() == 0 {
        return None;
    }

    // The bad byte map essentially encodes the position of the last occurrence
    // of each character in s2. We can get back the position by doing
    // (s2.len() - 1 - s2_bad_byte_map[c]). This gives the index in s2 of the
    // last occurrence of c.

    let mut longest_possible_subslice = cmp::min(s1.len(), s2.len());
    let mut i = 1;
    while i <= longest_possible_subslice {
        let c = s1[s1.len() - i] as usize;
        if s2_byte_map[c] as usize == s2.len() {
            // if c does not occur in s2
            if i - 1 < longest_possible_subslice {
                longest_possible_subslice = i - 1;
            }
            break;
        }
        let last_occurrence = s2.len() - 1 - s2_byte_map[c] as usize;
        // Length of the *longest* possible subslice which has c at the given
        // position
        let longest_subslice = last_occurrence + i;
        if longest_subslice < longest_possible_subslice {
            longest_possible_subslice = longest_subslice;
        }

        i += 1;
    }

    for i in (1..=longest_possible_subslice).rev() {
        if s1.ends_with(&s2[..i]) {
            return Some(s1.len() - i);
        }
    }

    None
}

// Return whether or not s1 ends with a subslice of s2
fn ends_with_subslice(s1: &[u8], s2: &[u8], s2_byte_map: &[u8]) -> bool {
    find_ending_subslice_of(s1, s2, s2_byte_map).is_some()
}

// Return the possible ending for the current value, either because the
// boundary is present in the current buffer, or a subslice of it is and it's
// possible that it will be completed on the next parse. If the value is not
// terminated within the contents of `buf`, the length of `buf` is returned.
fn find_value_len<B1, B2>(buf: B1, boundary: B2, boundary_byte_map: &[u8]) -> usize
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

// Map for implementing the Boyer-Moore bad character rule. Given a byte string
// `r` and a search string `s` where we want to find the position of an
// occurrence of `s` in `r`, the return value of this function tells us how many
// bytes we can skip over before reaching a possible occurrence of `s`.
//
// If we want to check if `s` starts at index `i` in `r`, we can look up
// `byte_map[r[i + s.len() - 1]]`. This value is the number of characters we
// can safely skip over before the next position where `s` might occur. We only
// have to check if `s` occurs at `i` if the value of the byte map is 0.
//
// https://en.wikipedia.org/wiki/Boyer%E2%80%93Moore_string-search_algorithm#The_bad-character_rule
pub const fn byte_map(s: &[u8]) -> Option<[u8; u8::MAX as usize + 1]> {
    if s.len() > u8::MAX as usize {
        return None;
    }

    let mut map = [s.len() as u8; u8::MAX as usize + 1];
    let mut i = 0;

    while i < s.len() {
        map[s[i] as usize] = (s.len() - (i + 1)) as u8;
        i += 1;
    }

    Some(map)
}

#[cfg(test)]
mod tests {
    use crate::multipart_form::*;

    #[test]
    fn test_find_subslice() {
        let s1 = vec![1, 2, 3, 3, 2, 5, 1];

        assert_eq!(find_subslice(&s1, &vec![3, 2], &byte_map(&vec![3, 2]).unwrap()), Some(3));
        assert_eq!(find_subslice(&s1, &vec![5, 5], &byte_map(&vec![5, 5]).unwrap()), None);
        assert_eq!(find_subslice(&s1, &vec![1], &byte_map(&vec![1]).unwrap()), Some(0));
        assert_eq!(find_subslice(&s1, &vec![0; 20], &byte_map(&vec![0; 20]).unwrap()), None);
        assert_eq!(find_subslice(&s1, &s1, &byte_map(&s1).unwrap()), Some(0));
    }

    #[test]
    fn test_find_ending_subslice_of() {
        let s1 = b"foobar";

        assert_eq!(find_ending_subslice_of(s1, b"barnacle", &byte_map(b"barnacle").unwrap()), Some(3));
        assert_eq!(find_ending_subslice_of(s1, b"foobar", &byte_map(b"foobar").unwrap()), Some(0));
        assert_eq!(find_ending_subslice_of(s1, b"foo", &byte_map(b"foo").unwrap()), None);
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
        let byte_map = byte_map(BOUNDARY).unwrap();
        const CD_1: &'static str = "form-data; name=\"field1\"";
        const VALUE_1: &'static [u8] = b"value1";
        const CD_2: &'static str = "form-data; name=\"field2\"; filename=\"example.txt\"";
        const VALUE_2: &'static [u8] = b"value2";

        let mut i = 0;
        let mut value = 0;
        loop {
            match parse(&FORM_BODY[i..], BOUNDARY, &byte_map) {
                ParseResult::NewValue(len, cd, _ct, val) => {
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
                ParseResult::Continue(val) => i += val.len(),
                ParseResult::Finished | ParseResult::Error | ParseResult::NeedMoreData => break
            }
        }
    }
}
