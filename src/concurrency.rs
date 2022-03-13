use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use crate::db::*;


// Count the number of concurrent accessors to files so that they are not
// deleted during downloads, etc.

pub struct Accessor {
    parent: Accessors,
    pub id: i64,
    pub mtx: Arc<Mutex<()>>,
    db_connection_info: DbConnectionInfo
}

impl Accessor {
    // Decrement the reference count
    pub fn revoke(&mut self) {
        let db_connection = establish_connection_info(&self.db_connection_info);
        Upload::revoke(&db_connection, self.id)
            .expect("Revoking access in DB");
        let mut map = self.parent.0.lock().unwrap();
        let (rc, _) = map.get_mut(&self.id).unwrap();
        *rc -= 1;
        if *rc == 0 {
            map.remove(&self.id);
        }
    }

    // Return whether or not there are other accessors on the same ID as is
    // possessed by this instance
    pub fn is_only_accessor(&self) -> bool {
        let db_connection = establish_connection_info(&self.db_connection_info);
        let map = self.parent.0.lock().unwrap();
        let (rc, _) = map.get(&self.id).unwrap();
        *rc == 1 && Upload::num_accessors(&db_connection, self.id) == Some(1)
    }
}


#[derive(Clone)]
pub struct Accessors (Arc<Mutex<HashMap<i64, (usize, Arc<Mutex<()>>)>>>);

impl Accessors {
    pub fn new() -> Self {
        Self (Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn access(&self, id: i64, db_connection_info: DbConnectionInfo) -> Accessor {
        let db_connection = establish_connection_info(&db_connection_info);
        Upload::access(&db_connection, id)
            .expect("Gaining access in DB");
        let mut map = self.0.lock().unwrap();
        match map.get_mut(&id) {
            Some((rc, mtx)) => {
                // Set a new mutex if the current one is poisoned
                let mtx = if mtx.lock().is_ok() {
                    mtx.clone()
                } else {
                    Arc::new(Mutex::new(()))
                };
                *rc += 1;
                let accessor = Accessor {
                    parent: self.clone(),
                    id,
                    mtx,
                    db_connection_info
                };

                accessor
            },
            None => {
                let mtx = Arc::new(Mutex::new(()));
                map.insert(id, (1, mtx.clone()));

                let accessor = Accessor {
                    parent: self.clone(),
                    id,
                    mtx,
                    db_connection_info
                };

                accessor
            }
        }
    }
}
