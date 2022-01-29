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

    // Increment the count for the given file. If `only_if_first` is true, only
    // increment if the count for `file` is not defined or 0. Returns whether or
    // not the count was incremented.
    pub fn increment(&self, file: String, only_if_first: bool) -> bool {
        let mut map = self.0.lock().unwrap();
        match map.get_mut(&file) {
            Some(rc) => {
                if only_if_first && *rc > 0 {
                    false
                } else {
                    *rc += 1;
                    true
                }
            },
            None => { map.insert(file, 1); true }
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

    pub fn num_accessors(&self, file: &str) -> usize {
        match self.0.lock().unwrap().get(file) {
            Some(num) => *num,
            None => 0
        }
    }
}
