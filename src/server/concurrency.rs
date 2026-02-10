use std::sync::{Arc, RwLock, RwLockReadGuard};
use dashmap::{DashMap, mapref::entry::Entry};


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
    id: i64,
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
        let map = &self.parent.0;
        map.remove_if(&self.id, |_, _| {
            let mut accessor = self.mtx.write().unwrap();
            accessor.rc -= 1;
            accessor.rc == 0
        });
    }
}


#[derive(Clone)]
pub struct Accessors (Arc<DashMap<i64, Arc<RwLock<Accessor>>>>);

impl Accessors {
    pub fn new() -> Self {
        Self (Arc::new(DashMap::new()))
    }

    pub fn access(&self, id: i64) -> AccessorMutex {
        let map = &self.0;
        let mtx = match map.entry(id) {
            Entry::Occupied(e) => {
                let mtx = e.get().clone();
                {
                    let mut accessor = mtx.write().unwrap();
                    accessor.rc += 1;
                }
                mtx
            },
            Entry::Vacant(e) => {
                let inner = Accessor {
                    id,
                    rc: 1
                };
                let mtx = Arc::new(RwLock::new(inner));
                e.insert(mtx.clone());
                mtx
            }
        };

        AccessorMutex {
            id: id,
            mtx: mtx,
            parent: self.clone()
        }
    }
}
