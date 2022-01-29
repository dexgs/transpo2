use rand::prelude::*;

// Build a string by randomly concatenating `len` elements of `pieces`
pub fn random_string<S>(pieces: &[S], len: usize) -> Option<String>
where S: AsRef<str>
{
    let mut rng = rand::thread_rng();
    let max_piece_len = pieces.iter().map(|s| s.as_ref().len()).max()?;
    let mut string = String::with_capacity(max_piece_len * len);

    for _ in 0..len {
        string.push_str(pieces[rng.gen_range(0..pieces.len())].as_ref());
    }

    return Some(string);
}
