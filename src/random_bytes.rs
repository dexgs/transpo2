use rand::prelude::*;

pub fn random_bytes(bytes: &mut [u8]) {
    let mut rng = rand::rng();
    for i in 0..bytes.len() {
        bytes[i] = rng.random();
    }
}
