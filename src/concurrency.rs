use std::sync::{Arc, RwLock, RwLockReadGuard};
use dashmap::DashMap;


// Count the number of concurrent accessors to files to make sure that they
// aren't deleted while being downloaded over a different connection.

pub struct Accessor {
    pub id: i64,
    rc: usize,
}

impl Accessor {
    // Return whether or not there are other accessors on the same ID as is
    // possessed by this instance
    pub fn is_only_accessor(&self) -> bool {
        self.rc == 1
    }
}

pub struct AccessorMutex {
    mtx: Arc<RwLock<Accessor>>,
    parent: Accessors
}

impl AccessorMutex {
    pub fn lock<'a>(&'a self) -> RwLockReadGuard<'a, Accessor> {
        self.mtx.read().unwrap()
    }
}

impl Drop for AccessorMutex {
    fn drop(&mut self) {
        let mut accessor = self.mtx.write().unwrap();

        accessor.rc -= 1;
        if accessor.rc == 0 {
            let map = &self.parent.0;
            map.remove(&accessor.id);
        }
    }
}


#[derive(Clone)]
pub struct Accessors (Arc<DashMap<i64, Arc<RwLock<Accessor>>>>);

impl Accessors {
    pub fn new() -> Self {
        Self (Arc::new((DashMap::new())))
    }

    pub fn access(&self, id: i64) -> AccessorMutex {
        let map = &self.0;

        // Get the existing mutex, or create it if it does not exist (or is poisoned)
        let accessor_mutex = match map.get(&id) {
            Some(accessor_mutex) => {
                match accessor_mutex.write() {
                    Ok(mut accessor) => {
                        accessor.rc += 1;
                        accessor_mutex.clone()
                    }
                    Err(_) => {
                        // Handle a poisoned lock...
                        let accessor = Accessor {
                            id,
                            rc: 1
                        };
                        let accessor_mutex = Arc::new(RwLock::new(accessor));
                        accessor_mutex
                    }
                }
            },
            None => {
                let accessor = Accessor {
                    id,
                    rc: 1
                };
                let accessor_mutex = Arc::new(RwLock::new(accessor));
                accessor_mutex
            }
        };

        map.insert(id, accessor_mutex.clone());

        AccessorMutex {
            mtx: accessor_mutex,
            parent: self.clone()
        }
    }
}
