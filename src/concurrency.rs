use std::sync::{Arc, Mutex};
use std::collections::HashMap;


// Count the number of concurrent accessors to files so that they are not
// deleted during downloads, etc.

#[derive(Clone)]
pub struct Accessors (Arc<Mutex<HashMap<String, usize>>>);

impl Accessors {
    pub fn new() -> Self {
        Self (Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn increment(&self, file: String) {
        let mut map = self.0.lock().unwrap();
        match map.get_mut(&file) {
            Some(rc) => *rc += 1,
            None => { map.insert(file, 1); }
        }
    }

    // Returns whether or not decrementing reduces the count to 0 for `file`
    pub fn decrement(&self, file: &str) -> bool {
        let mut map = self.0.lock().unwrap();
        let zero_accessors = match map.get_mut(file) {
            Some(rc) => {
                *rc -= 1;
                *rc == 0
            },
            None => true
        };

        if zero_accessors {
            map.remove(file);
        }

        zero_accessors
    }
}
