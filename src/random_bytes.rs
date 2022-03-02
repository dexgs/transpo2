use rand::prelude::*;

pub fn random_bytes(bytes: &mut [u8]) {
    let mut rng = rand::thread_rng();
    for i in 0..bytes.len() {
        bytes[i] = rng.gen();
    }
}
