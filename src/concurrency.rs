use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use crate::db::*;


// Count the number of concurrent accessors to files to make sure that they
// aren't deleted while being downloaded over a different connection.

pub struct Accessor {
    parent: Accessors,
    pub id: i64,
    rc: usize,
    db_connection_info: DbConnectionInfo
}

impl Accessor {
    // Decrement the reference count
    pub fn revoke(&mut self) {
        let db_connection = establish_connection_info(&self.db_connection_info);
        Upload::revoke(&db_connection, self.id)
            .expect("Revoking access in DB");

        self.rc -= 1;
        if self.rc == 0 {
            let mut map = self.parent.0.lock().unwrap();
            map.remove(&self.id);
        }
    }

    // Return whether or not there are other accessors on the same ID as is
    // possessed by this instance
    pub fn is_only_accessor(&self) -> bool {
        let db_connection = establish_connection_info(&self.db_connection_info);
        self.rc == 1 && Upload::num_accessors(&db_connection, self.id) == Some(1)
    }
}


#[derive(Clone)]
pub struct Accessors (Arc<Mutex<HashMap<i64, Arc<Mutex<Accessor>>>>>);

impl Accessors {
    pub fn new() -> Self {
        Self (Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn access(&self, id: i64, db_connection_info: DbConnectionInfo) -> Arc<Mutex<Accessor>> {
        let db_connection = establish_connection_info(&db_connection_info);
        Upload::access(&db_connection, id)
            .expect("Gaining access in DB");

        let mut map = self.0.lock().unwrap();

        // Get the existing mutex, or create it if it does not exist (or is poisoned)
        let accessor_mutex = match map.get(&id) {
            Some(accessor_mutex) => {
                match accessor_mutex.lock() {
                    Ok(mut accessor) => {
                        accessor.rc += 1;
                        accessor_mutex.clone()
                    }
                    Err(_) => {
                        // Handle a poisoned lock...
                        let accessor = Accessor {
                            parent: self.clone(),
                            id,
                            rc: 1,
                            db_connection_info
                        };
                        let accessor_mutex = Arc::new(Mutex::new(accessor));
                        accessor_mutex
                    }
                }
            },
            None => {
                let accessor = Accessor {
                    parent: self.clone(),
                    id,
                    rc: 1,
                    db_connection_info
                };
                let accessor_mutex = Arc::new(Mutex::new(accessor));
                accessor_mutex
            }
        };

        map.insert(id, accessor_mutex.clone());
        accessor_mutex
    }
}
