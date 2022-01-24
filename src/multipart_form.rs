use std::cmp;

const CD: &'static [u8] = b"Content-Disposition: ";
const TERMINATOR: &'static [u8] = b"--";
const NEWLINE: &'static [u8] = b"\n";
const DOUBLE_NEWLINE: &'static [u8] = b"\n\n";

pub enum ParseResult<'a> {
    // Contains ref to Content-Disposition string
    NewValue(&'a str, &'a [u8]),
    Continue(&'a [u8]),
    Finished,
    Error
}

// Returns the length of the data parsed, what was parsed and, if it is a new
// form field, the Content-Disposition.
//
// Subsequent calls to this function MUST guarantee that `buf` begins where
// parsing last ended.
//
// `boundary` is assumed to be prefixed with "--"
pub fn parse<'a, B1, B2>(buf: B1, boundary: B2) -> ()//(usize, ParseResult<'a>)
where B1: AsRef<[u8]> + 'a,
      B2: AsRef<[u8]>
{
    let buf = buf.as_ref();
    let boundary = boundary.as_ref();

    if let Some(buf) = buf.strip_prefix(boundary) {
        // This is either the end of the form or the start of a new form field
        if buf.starts_with(TERMINATOR) {
            // This is the end of the form
            // return (0, ParseType::Finished);
        } else if let Some((buf, cd_len)) = buf
            .strip_prefix(NEWLINE)
            .and_then(|buf| buf.strip_prefix(CD))
            .and_then(|buf| Some((buf, find_subslice(buf, DOUBLE_NEWLINE)?)))
        {
            // This is a new field in the form
        } else {
            // The form is improperly formatted
            // return (0, ParseType::Error);
        }
    } else {
        // This is the continuation of the value of the previous field
    }
}

// Return the index of the first instance of s2 in s1, or None if there isn't any
fn find_subslice<T>(s1: &[T], s2: &[T]) -> Option<usize>
where T: PartialEq
{
    for i in 0..s1.len() {
        if i + s2.len() > s1.len() {
            return None;
        } else if &s1[i..(i + s2.len())] == s2 {
            return Some(i);
        }
    }

    None
}

// Return the index at which a subslice of s2 (must equal s2[0..n] for any n)
// occurs at the end of s1.
//
// Example: for s1 = "foobar" and s2 = "barnacle", the functioun should return 3
fn find_ending_subslice_of<T>(s1: &[T], s2: &[T]) -> Option<usize>
where T: PartialEq
{
    if s1.len() == 0 || s2.len() == 0 {
        return None;
    }

    for sub_len in 1..=cmp::min(s2.len(), s1.len()) {
        if s1.ends_with(&s2[..sub_len]) {
            return Some(s1.len() - sub_len);
        } else {
            find_subslice(s2, &s1[(s1.len() - sub_len)..])?;
        }
    }

    None
}

// Return the possible ending for the current value, either because the
// boundary is present in the current buffer, or a subslice of it is and it's
// possible that it will be completed on the next parse. If the value is not
// terminated within the contents of `buf`, the length of `buf` is returned.
fn find_value_len<B1, B2>(buf: B1, boundary: B2) -> usize
where B1: AsRef<[u8]>,
      B2: AsRef<[u8]>
{
    let buf = buf.as_ref();
    let boundary = boundary.as_ref();

    if let Some(i) = find_subslice(buf, boundary) {
        i
    } else if let Some(i) = find_ending_subslice_of(buf, boundary) {
        i
    } else {
        buf.len()
    }
}


#[cfg(test)]
mod tests {
    use crate::multipart_form::*;

    const FORM_BODY: &'static [u8] =
b"--boundary
Content-Disposition: form-data; name=\"field1\"

value1
--boundary
Content-Disposition: form-data; name=\"field2\"; filename=\"example.txt\"

value2
--boundary--";


    #[test]
    fn test_find_subslice() {
        let s1 = vec![1, 2, 3, 3, 2, 5, 1];

        assert_eq!(find_subslice(&s1, &vec![3, 2]), Some(3));
        assert_eq!(find_subslice(&s1, &vec![5, 5]), None);
        assert_eq!(find_subslice(&s1, &vec![1]), Some(0));
        assert_eq!(find_subslice(&s1, &vec![0; 20]), None);
        assert_eq!(find_subslice(&s1, &s1), Some(0));
    }

    #[test]
    fn test_find_ending_subslice_of() {
        let s1 = b"foobar";

        assert_eq!(find_ending_subslice_of(s1, b"barnacle"), Some(3));
        assert_eq!(find_ending_subslice_of(s1, b"foobar"), Some(0));
        assert_eq!(find_ending_subslice_of(s1, b"foo"), None);
    }
}
